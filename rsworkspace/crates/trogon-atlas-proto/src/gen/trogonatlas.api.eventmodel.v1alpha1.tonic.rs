// @generated
/// Generated client implementations.
pub mod event_model_service_client {
    #![allow(unused_variables, dead_code, missing_docs, clippy::let_unit_value)]
    use tonic::codegen::*;
    use tonic::codegen::http::Uri;
    ///
    #[derive(Debug, Clone)]
    pub struct EventModelServiceClient<T> {
        inner: tonic::client::Grpc<T>,
    }
    impl EventModelServiceClient<tonic::transport::Channel> {
        /// Attempt to create a new client by connecting to a given endpoint.
        pub async fn connect<D>(dst: D) -> Result<Self, tonic::transport::Error>
        where
            D: TryInto<tonic::transport::Endpoint>,
            D::Error: Into<StdError>,
        {
            let conn = tonic::transport::Endpoint::new(dst)?.connect().await?;
            Ok(Self::new(conn))
        }
    }
    impl<T> EventModelServiceClient<T>
    where
        T: tonic::client::GrpcService<tonic::body::BoxBody>,
        T::Error: Into<StdError>,
        T::ResponseBody: Body<Data = Bytes> + Send + 'static,
        <T::ResponseBody as Body>::Error: Into<StdError> + Send,
    {
        pub fn new(inner: T) -> Self {
            let inner = tonic::client::Grpc::new(inner);
            Self { inner }
        }
        pub fn with_origin(inner: T, origin: Uri) -> Self {
            let inner = tonic::client::Grpc::with_origin(inner, origin);
            Self { inner }
        }
        pub fn with_interceptor<F>(
            inner: T,
            interceptor: F,
        ) -> EventModelServiceClient<InterceptedService<T, F>>
        where
            F: tonic::service::Interceptor,
            T::ResponseBody: Default,
            T: tonic::codegen::Service<
                http::Request<tonic::body::BoxBody>,
                Response = http::Response<
                    <T as tonic::client::GrpcService<tonic::body::BoxBody>>::ResponseBody,
                >,
            >,
            <T as tonic::codegen::Service<
                http::Request<tonic::body::BoxBody>,
            >>::Error: Into<StdError> + Send + Sync,
        {
            EventModelServiceClient::new(InterceptedService::new(inner, interceptor))
        }
        /// Compress requests with the given encoding.
        ///
        /// This requires the server to support it otherwise it might respond with an
        /// error.
        #[must_use]
        pub fn send_compressed(mut self, encoding: CompressionEncoding) -> Self {
            self.inner = self.inner.send_compressed(encoding);
            self
        }
        /// Enable decompressing responses.
        #[must_use]
        pub fn accept_compressed(mut self, encoding: CompressionEncoding) -> Self {
            self.inner = self.inner.accept_compressed(encoding);
            self
        }
        /// Limits the maximum size of a decoded message.
        ///
        /// Default: `4MB`
        #[must_use]
        pub fn max_decoding_message_size(mut self, limit: usize) -> Self {
            self.inner = self.inner.max_decoding_message_size(limit);
            self
        }
        /// Limits the maximum size of an encoded message.
        ///
        /// Default: `usize::MAX`
        #[must_use]
        pub fn max_encoding_message_size(mut self, limit: usize) -> Self {
            self.inner = self.inner.max_encoding_message_size(limit);
            self
        }
        /** Bootstrap. Clients SHOULD call GetServerInfo on connect to learn
 which optional features and limits apply (Decision #22).
*/
        pub async fn get_server_info(
            &mut self,
            request: impl tonic::IntoRequest<super::GetServerInfoRequest>,
        ) -> std::result::Result<
            tonic::Response<super::GetServerInfoResponse>,
            tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::new(
                        tonic::Code::Unknown,
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic::codec::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/GetServerInfo",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "trogonatlas.api.eventmodel.v1alpha1.EventModelService",
                        "GetServerInfo",
                    ),
                );
            self.inner.unary(req, path, codec).await
        }
        ///
        pub async fn list_namespaces(
            &mut self,
            request: impl tonic::IntoRequest<super::ListNamespacesRequest>,
        ) -> std::result::Result<
            tonic::Response<super::ListNamespacesResponse>,
            tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::new(
                        tonic::Code::Unknown,
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic::codec::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/ListNamespaces",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "trogonatlas.api.eventmodel.v1alpha1.EventModelService",
                        "ListNamespaces",
                    ),
                );
            self.inner.unary(req, path, codec).await
        }
        /** Caller identity. Reader-level: any authenticated principal may learn
 its own role, namespaces, and owner boundary.
*/
        pub async fn who_am_i(
            &mut self,
            request: impl tonic::IntoRequest<super::WhoAmIRequest>,
        ) -> std::result::Result<tonic::Response<super::WhoAmIResponse>, tonic::Status> {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::new(
                        tonic::Code::Unknown,
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic::codec::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/WhoAmI",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "trogonatlas.api.eventmodel.v1alpha1.EventModelService",
                        "WhoAmI",
                    ),
                );
            self.inner.unary(req, path, codec).await
        }
        /** Namespace registry. Registering claims a human name under an ownership
 node and mints the immutable id that storage keys use; moving re-parents
 an existing namespace without touching a single entity.
*/
        pub async fn register_namespace(
            &mut self,
            request: impl tonic::IntoRequest<super::RegisterNamespaceRequest>,
        ) -> std::result::Result<
            tonic::Response<super::RegisterNamespaceResponse>,
            tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::new(
                        tonic::Code::Unknown,
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic::codec::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/RegisterNamespace",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "trogonatlas.api.eventmodel.v1alpha1.EventModelService",
                        "RegisterNamespace",
                    ),
                );
            self.inner.unary(req, path, codec).await
        }
        ///
        pub async fn move_namespace(
            &mut self,
            request: impl tonic::IntoRequest<super::MoveNamespaceRequest>,
        ) -> std::result::Result<
            tonic::Response<super::MoveNamespaceResponse>,
            tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::new(
                        tonic::Code::Unknown,
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic::codec::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/MoveNamespace",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "trogonatlas.api.eventmodel.v1alpha1.EventModelService",
                        "MoveNamespace",
                    ),
                );
            self.inner.unary(req, path, codec).await
        }
        /** Content-addressable identity of the current state. Branch-scoped via
 the `x-trogon-atlas-branch` header like every other read.
*/
        pub async fn get_snapshot_id(
            &mut self,
            request: impl tonic::IntoRequest<super::GetSnapshotIdRequest>,
        ) -> std::result::Result<
            tonic::Response<super::GetSnapshotIdResponse>,
            tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::new(
                        tonic::Code::Unknown,
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic::codec::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/GetSnapshotId",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "trogonatlas.api.eventmodel.v1alpha1.EventModelService",
                        "GetSnapshotId",
                    ),
                );
            self.inner.unary(req, path, codec).await
        }
        /** Direct lookup.
*/
        pub async fn get_entity(
            &mut self,
            request: impl tonic::IntoRequest<super::GetEntityRequest>,
        ) -> std::result::Result<
            tonic::Response<super::GetEntityResponse>,
            tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::new(
                        tonic::Code::Unknown,
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic::codec::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/GetEntity",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "trogonatlas.api.eventmodel.v1alpha1.EventModelService",
                        "GetEntity",
                    ),
                );
            self.inner.unary(req, path, codec).await
        }
        ///
        pub async fn batch_get_entities(
            &mut self,
            request: impl tonic::IntoRequest<super::BatchGetEntitiesRequest>,
        ) -> std::result::Result<
            tonic::Response<super::BatchGetEntitiesResponse>,
            tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::new(
                        tonic::Code::Unknown,
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic::codec::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/BatchGetEntities",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "trogonatlas.api.eventmodel.v1alpha1.EventModelService",
                        "BatchGetEntities",
                    ),
                );
            self.inner.unary(req, path, codec).await
        }
        /** Discovery.
*/
        pub async fn list_entities(
            &mut self,
            request: impl tonic::IntoRequest<super::ListEntitiesRequest>,
        ) -> std::result::Result<
            tonic::Response<super::ListEntitiesResponse>,
            tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::new(
                        tonic::Code::Unknown,
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic::codec::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/ListEntities",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "trogonatlas.api.eventmodel.v1alpha1.EventModelService",
                        "ListEntities",
                    ),
                );
            self.inner.unary(req, path, codec).await
        }
        ///
        pub async fn search_entities(
            &mut self,
            request: impl tonic::IntoRequest<super::SearchEntitiesRequest>,
        ) -> std::result::Result<
            tonic::Response<super::SearchEntitiesResponse>,
            tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::new(
                        tonic::Code::Unknown,
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic::codec::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/SearchEntities",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "trogonatlas.api.eventmodel.v1alpha1.EventModelService",
                        "SearchEntities",
                    ),
                );
            self.inner.unary(req, path, codec).await
        }
        /** Versioning.
*/
        pub async fn list_versions(
            &mut self,
            request: impl tonic::IntoRequest<super::ListVersionsRequest>,
        ) -> std::result::Result<
            tonic::Response<super::ListVersionsResponse>,
            tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::new(
                        tonic::Code::Unknown,
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic::codec::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/ListVersions",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "trogonatlas.api.eventmodel.v1alpha1.EventModelService",
                        "ListVersions",
                    ),
                );
            self.inner.unary(req, path, codec).await
        }
        ///
        pub async fn get_latest_version(
            &mut self,
            request: impl tonic::IntoRequest<super::GetLatestVersionRequest>,
        ) -> std::result::Result<
            tonic::Response<super::GetLatestVersionResponse>,
            tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::new(
                        tonic::Code::Unknown,
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic::codec::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/GetLatestVersion",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "trogonatlas.api.eventmodel.v1alpha1.EventModelService",
                        "GetLatestVersion",
                    ),
                );
            self.inner.unary(req, path, codec).await
        }
        ///
        pub async fn get_supersession_chain(
            &mut self,
            request: impl tonic::IntoRequest<super::GetSupersessionChainRequest>,
        ) -> std::result::Result<
            tonic::Response<super::GetSupersessionChainResponse>,
            tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::new(
                        tonic::Code::Unknown,
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic::codec::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/GetSupersessionChain",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "trogonatlas.api.eventmodel.v1alpha1.EventModelService",
                        "GetSupersessionChain",
                    ),
                );
            self.inner.unary(req, path, codec).await
        }
        /** Graph traversal across typed Refs/Edges.
*/
        pub async fn get_incoming_references(
            &mut self,
            request: impl tonic::IntoRequest<super::GetReferencesRequest>,
        ) -> std::result::Result<
            tonic::Response<super::GetReferencesResponse>,
            tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::new(
                        tonic::Code::Unknown,
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic::codec::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/GetIncomingReferences",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "trogonatlas.api.eventmodel.v1alpha1.EventModelService",
                        "GetIncomingReferences",
                    ),
                );
            self.inner.unary(req, path, codec).await
        }
        ///
        pub async fn get_outgoing_references(
            &mut self,
            request: impl tonic::IntoRequest<super::GetReferencesRequest>,
        ) -> std::result::Result<
            tonic::Response<super::GetReferencesResponse>,
            tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::new(
                        tonic::Code::Unknown,
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic::codec::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/GetOutgoingReferences",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "trogonatlas.api.eventmodel.v1alpha1.EventModelService",
                        "GetOutgoingReferences",
                    ),
                );
            self.inner.unary(req, path, codec).await
        }
        /** Transitive reverse-reference closure: "what depends on this?"
*/
        pub async fn get_impact(
            &mut self,
            request: impl tonic::IntoRequest<super::GetImpactRequest>,
        ) -> std::result::Result<
            tonic::Response<super::GetImpactResponse>,
            tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::new(
                        tonic::Code::Unknown,
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic::codec::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/GetImpact",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "trogonatlas.api.eventmodel.v1alpha1.EventModelService",
                        "GetImpact",
                    ),
                );
            self.inner.unary(req, path, codec).await
        }
        /** Rewrite every stored reference to `from` so it points at `to`. Used as
 the rewire step in the promotion workflow: create v+1 with supersedes,
 call retarget_references (dry_run first to preview), then drain any
 residual SLICE_STALE_REF findings. For partial migrations or when only
 a subset of referrers should move, fall back to get_incoming_references
 + batch_mutate.
*/
        pub async fn retarget_references(
            &mut self,
            request: impl tonic::IntoRequest<super::RetargetReferencesRequest>,
        ) -> std::result::Result<
            tonic::Response<super::RetargetReferencesResponse>,
            tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::new(
                        tonic::Code::Unknown,
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic::codec::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/RetargetReferences",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "trogonatlas.api.eventmodel.v1alpha1.EventModelService",
                        "RetargetReferences",
                    ),
                );
            self.inner.unary(req, path, codec).await
        }
        /** Denormalized projections. These are the primary RPCs for MCP tools.
*/
        pub async fn get_slice_projection(
            &mut self,
            request: impl tonic::IntoRequest<super::GetSliceProjectionRequest>,
        ) -> std::result::Result<
            tonic::Response<super::GetSliceProjectionResponse>,
            tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::new(
                        tonic::Code::Unknown,
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic::codec::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/GetSliceProjection",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "trogonatlas.api.eventmodel.v1alpha1.EventModelService",
                        "GetSliceProjection",
                    ),
                );
            self.inner.unary(req, path, codec).await
        }
        ///
        pub async fn get_storyboard_projection(
            &mut self,
            request: impl tonic::IntoRequest<super::GetStoryboardProjectionRequest>,
        ) -> std::result::Result<
            tonic::Response<super::GetStoryboardProjectionResponse>,
            tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::new(
                        tonic::Code::Unknown,
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic::codec::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/GetStoryboardProjection",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "trogonatlas.api.eventmodel.v1alpha1.EventModelService",
                        "GetStoryboardProjection",
                    ),
                );
            self.inner.unary(req, path, codec).await
        }
        ///
        pub async fn get_event_model_projection(
            &mut self,
            request: impl tonic::IntoRequest<super::GetEventModelProjectionRequest>,
        ) -> std::result::Result<
            tonic::Response<super::GetEventModelProjectionResponse>,
            tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::new(
                        tonic::Code::Unknown,
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic::codec::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/GetEventModelProjection",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "trogonatlas.api.eventmodel.v1alpha1.EventModelService",
                        "GetEventModelProjection",
                    ),
                );
            self.inner.unary(req, path, codec).await
        }
        /** Closure-as-new-model: extract a slice/storyboard's reachable graph
 into a fresh, self-contained EventModel.
*/
        pub async fn extract_subgraph(
            &mut self,
            request: impl tonic::IntoRequest<super::ExtractSubgraphRequest>,
        ) -> std::result::Result<
            tonic::Response<super::ExtractSubgraphResponse>,
            tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::new(
                        tonic::Code::Unknown,
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic::codec::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/ExtractSubgraph",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "trogonatlas.api.eventmodel.v1alpha1.EventModelService",
                        "ExtractSubgraph",
                    ),
                );
            self.inner.unary(req, path, codec).await
        }
        /** Structural comparison of two entities (typically two versions).
*/
        pub async fn diff_entities(
            &mut self,
            request: impl tonic::IntoRequest<super::DiffEntitiesRequest>,
        ) -> std::result::Result<
            tonic::Response<super::DiffEntitiesResponse>,
            tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::new(
                        tonic::Code::Unknown,
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic::codec::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/DiffEntities",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "trogonatlas.api.eventmodel.v1alpha1.EventModelService",
                        "DiffEntities",
                    ),
                );
            self.inner.unary(req, path, codec).await
        }
        /** Mutation. PutEntity / DeleteEntity are single-entity. BatchMutate is
 atomic across an ordered list of ops (Decision #23).
*/
        pub async fn put_entity(
            &mut self,
            request: impl tonic::IntoRequest<super::PutEntityRequest>,
        ) -> std::result::Result<
            tonic::Response<super::PutEntityResponse>,
            tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::new(
                        tonic::Code::Unknown,
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic::codec::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/PutEntity",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "trogonatlas.api.eventmodel.v1alpha1.EventModelService",
                        "PutEntity",
                    ),
                );
            self.inner.unary(req, path, codec).await
        }
        ///
        pub async fn delete_entity(
            &mut self,
            request: impl tonic::IntoRequest<super::DeleteEntityRequest>,
        ) -> std::result::Result<
            tonic::Response<super::DeleteEntityResponse>,
            tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::new(
                        tonic::Code::Unknown,
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic::codec::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/DeleteEntity",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "trogonatlas.api.eventmodel.v1alpha1.EventModelService",
                        "DeleteEntity",
                    ),
                );
            self.inner.unary(req, path, codec).await
        }
        ///
        pub async fn batch_mutate(
            &mut self,
            request: impl tonic::IntoRequest<super::BatchMutateRequest>,
        ) -> std::result::Result<
            tonic::Response<super::BatchMutateResponse>,
            tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::new(
                        tonic::Code::Unknown,
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic::codec::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/BatchMutate",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "trogonatlas.api.eventmodel.v1alpha1.EventModelService",
                        "BatchMutate",
                    ),
                );
            self.inner.unary(req, path, codec).await
        }
        /** Poll-based change feed (Decision #19: unary, not streaming).
*/
        pub async fn list_changes(
            &mut self,
            request: impl tonic::IntoRequest<super::ListChangesRequest>,
        ) -> std::result::Result<
            tonic::Response<super::ListChangesResponse>,
            tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::new(
                        tonic::Code::Unknown,
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic::codec::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/ListChanges",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "trogonatlas.api.eventmodel.v1alpha1.EventModelService",
                        "ListChanges",
                    ),
                );
            self.inner.unary(req, path, codec).await
        }
        /** Server-streaming change feed for non-MCP consumers. Same data as
 ListChanges, push semantics. MCP adapters should prefer ListChanges.
*/
        pub async fn stream_changes(
            &mut self,
            request: impl tonic::IntoRequest<super::StreamChangesRequest>,
        ) -> std::result::Result<
            tonic::Response<
                tonic::codec::Streaming<
                    super::super::super::super::catalog::v1alpha1::ChangeEvent,
                >,
            >,
            tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::new(
                        tonic::Code::Unknown,
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic::codec::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/StreamChanges",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "trogonatlas.api.eventmodel.v1alpha1.EventModelService",
                        "StreamChanges",
                    ),
                );
            self.inner.server_streaming(req, path, codec).await
        }
        /** Durable changeset log: what landed together, who did it, when. Unlike
 the change feed these records do not expire and are not dropped when a
 publish fails.
*/
        pub async fn list_changesets(
            &mut self,
            request: impl tonic::IntoRequest<super::ListChangesetsRequest>,
        ) -> std::result::Result<
            tonic::Response<super::ListChangesetsResponse>,
            tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::new(
                        tonic::Code::Unknown,
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic::codec::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/ListChangesets",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "trogonatlas.api.eventmodel.v1alpha1.EventModelService",
                        "ListChangesets",
                    ),
                );
            self.inner.unary(req, path, codec).await
        }
        ///
        pub async fn get_changeset(
            &mut self,
            request: impl tonic::IntoRequest<super::GetChangesetRequest>,
        ) -> std::result::Result<
            tonic::Response<super::GetChangesetResponse>,
            tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::new(
                        tonic::Code::Unknown,
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic::codec::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/GetChangeset",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "trogonatlas.api.eventmodel.v1alpha1.EventModelService",
                        "GetChangeset",
                    ),
                );
            self.inner.unary(req, path, codec).await
        }
        /** Per-entity edit log and undo. See docs/explanation/changesets.md.
*/
        pub async fn get_entity_history(
            &mut self,
            request: impl tonic::IntoRequest<super::GetEntityHistoryRequest>,
        ) -> std::result::Result<
            tonic::Response<super::GetEntityHistoryResponse>,
            tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::new(
                        tonic::Code::Unknown,
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic::codec::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/GetEntityHistory",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "trogonatlas.api.eventmodel.v1alpha1.EventModelService",
                        "GetEntityHistory",
                    ),
                );
            self.inner.unary(req, path, codec).await
        }
        ///
        pub async fn revert_changeset(
            &mut self,
            request: impl tonic::IntoRequest<super::RevertChangesetRequest>,
        ) -> std::result::Result<
            tonic::Response<super::RevertChangesetResponse>,
            tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::new(
                        tonic::Code::Unknown,
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic::codec::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/RevertChangeset",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "trogonatlas.api.eventmodel.v1alpha1.EventModelService",
                        "RevertChangeset",
                    ),
                );
            self.inner.unary(req, path, codec).await
        }
        /** Look up the receipt for an operation_id this caller claimed on a prior
 mutation call. Scoped to the calling principal.
*/
        pub async fn get_operation(
            &mut self,
            request: impl tonic::IntoRequest<super::GetOperationRequest>,
        ) -> std::result::Result<
            tonic::Response<super::GetOperationResponse>,
            tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::new(
                        tonic::Code::Unknown,
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic::codec::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/GetOperation",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "trogonatlas.api.eventmodel.v1alpha1.EventModelService",
                        "GetOperation",
                    ),
                );
            self.inner.unary(req, path, codec).await
        }
        /** Validation.
*/
        pub async fn validate_event_model(
            &mut self,
            request: impl tonic::IntoRequest<super::ValidateEventModelRequest>,
        ) -> std::result::Result<
            tonic::Response<super::ValidateEventModelResponse>,
            tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::new(
                        tonic::Code::Unknown,
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic::codec::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/ValidateEventModel",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "trogonatlas.api.eventmodel.v1alpha1.EventModelService",
                        "ValidateEventModel",
                    ),
                );
            self.inner.unary(req, path, codec).await
        }
        /** Returns the static catalog of every rule the validator can emit.
 Stable across runs; the only thing that changes is the codebase
 version. Use this to drive UI legends, doc links, and CI severity
 gates.
*/
        pub async fn list_validation_rules(
            &mut self,
            request: impl tonic::IntoRequest<super::ListValidationRulesRequest>,
        ) -> std::result::Result<
            tonic::Response<super::ListValidationRulesResponse>,
            tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::new(
                        tonic::Code::Unknown,
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic::codec::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/ListValidationRules",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "trogonatlas.api.eventmodel.v1alpha1.EventModelService",
                        "ListValidationRules",
                    ),
                );
            self.inner.unary(req, path, codec).await
        }
        /** Validate every EventModel in a project (or domain) in one call.
*/
        pub async fn validate_project(
            &mut self,
            request: impl tonic::IntoRequest<super::ValidateProjectRequest>,
        ) -> std::result::Result<
            tonic::Response<super::ValidateProjectResponse>,
            tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::new(
                        tonic::Code::Unknown,
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic::codec::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/ValidateProject",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "trogonatlas.api.eventmodel.v1alpha1.EventModelService",
                        "ValidateProject",
                    ),
                );
            self.inner.unary(req, path, codec).await
        }
        /** Bulk delete by structural query (project/namespace/kind/slug). One
 call replaces the "list-then-batch" client-side pattern.
*/
        pub async fn delete_by_query(
            &mut self,
            request: impl tonic::IntoRequest<super::DeleteByQueryRequest>,
        ) -> std::result::Result<
            tonic::Response<super::DeleteByQueryResponse>,
            tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::new(
                        tonic::Code::Unknown,
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic::codec::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/DeleteByQuery",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "trogonatlas.api.eventmodel.v1alpha1.EventModelService",
                        "DeleteByQuery",
                    ),
                );
            self.inner.unary(req, path, codec).await
        }
        /** Static catalog of every EntityKind the system stores.
*/
        pub async fn list_entity_kinds(
            &mut self,
            request: impl tonic::IntoRequest<super::ListEntityKindsRequest>,
        ) -> std::result::Result<
            tonic::Response<super::ListEntityKindsResponse>,
            tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::new(
                        tonic::Code::Unknown,
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic::codec::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/ListEntityKinds",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "trogonatlas.api.eventmodel.v1alpha1.EventModelService",
                        "ListEntityKinds",
                    ),
                );
            self.inner.unary(req, path, codec).await
        }
        /** List every entity scoped to a Domain (walks the EM->BC->Subdomain->Domain
 realizes chain server-side so clients don't have to).
*/
        pub async fn list_entities_by_domain(
            &mut self,
            request: impl tonic::IntoRequest<super::ListEntitiesByDomainRequest>,
        ) -> std::result::Result<
            tonic::Response<super::ListEntitiesByDomainResponse>,
            tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::new(
                        tonic::Code::Unknown,
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic::codec::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/ListEntitiesByDomain",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "trogonatlas.api.eventmodel.v1alpha1.EventModelService",
                        "ListEntitiesByDomain",
                    ),
                );
            self.inner.unary(req, path, codec).await
        }
        /** Analysis. The AI does the heavy lifting; humans review the gaps it
 surfaces. Neither RPC mutates the model.
*/
        pub async fn infer_data_flow(
            &mut self,
            request: impl tonic::IntoRequest<super::InferDataFlowRequest>,
        ) -> std::result::Result<
            tonic::Response<super::InferDataFlowResponse>,
            tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::new(
                        tonic::Code::Unknown,
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic::codec::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/InferDataFlow",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "trogonatlas.api.eventmodel.v1alpha1.EventModelService",
                        "InferDataFlow",
                    ),
                );
            self.inner.unary(req, path, codec).await
        }
        ///
        pub async fn check_information_completeness(
            &mut self,
            request: impl tonic::IntoRequest<super::CheckInformationCompletenessRequest>,
        ) -> std::result::Result<
            tonic::Response<super::CheckInformationCompletenessResponse>,
            tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::new(
                        tonic::Code::Unknown,
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic::codec::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/CheckInformationCompleteness",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "trogonatlas.api.eventmodel.v1alpha1.EventModelService",
                        "CheckInformationCompleteness",
                    ),
                );
            self.inner.unary(req, path, codec).await
        }
        /** Branch lifecycle (Phase 1: Isolation). See the "Branch context" note
 near the top of this file for how every other RPC picks up branch scope.
*/
        pub async fn create_branch(
            &mut self,
            request: impl tonic::IntoRequest<super::CreateBranchRequest>,
        ) -> std::result::Result<
            tonic::Response<super::CreateBranchResponse>,
            tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::new(
                        tonic::Code::Unknown,
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic::codec::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/CreateBranch",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "trogonatlas.api.eventmodel.v1alpha1.EventModelService",
                        "CreateBranch",
                    ),
                );
            self.inner.unary(req, path, codec).await
        }
        ///
        pub async fn list_branches(
            &mut self,
            request: impl tonic::IntoRequest<super::ListBranchesRequest>,
        ) -> std::result::Result<
            tonic::Response<super::ListBranchesResponse>,
            tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::new(
                        tonic::Code::Unknown,
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic::codec::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/ListBranches",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "trogonatlas.api.eventmodel.v1alpha1.EventModelService",
                        "ListBranches",
                    ),
                );
            self.inner.unary(req, path, codec).await
        }
        ///
        pub async fn delete_branch(
            &mut self,
            request: impl tonic::IntoRequest<super::DeleteBranchRequest>,
        ) -> std::result::Result<
            tonic::Response<super::DeleteBranchResponse>,
            tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::new(
                        tonic::Code::Unknown,
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic::codec::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/DeleteBranch",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "trogonatlas.api.eventmodel.v1alpha1.EventModelService",
                        "DeleteBranch",
                    ),
                );
            self.inner.unary(req, path, codec).await
        }
        /** Branch review and merge (Phase 2). `name` is an explicit request field,
 like the three lifecycle RPCs above, since these operate ON a named
 branch rather than being scoped BY a branch context.
*/
        pub async fn diff_branch(
            &mut self,
            request: impl tonic::IntoRequest<super::DiffBranchRequest>,
        ) -> std::result::Result<
            tonic::Response<super::DiffBranchResponse>,
            tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::new(
                        tonic::Code::Unknown,
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic::codec::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/DiffBranch",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "trogonatlas.api.eventmodel.v1alpha1.EventModelService",
                        "DiffBranch",
                    ),
                );
            self.inner.unary(req, path, codec).await
        }
        ///
        pub async fn merge_branch(
            &mut self,
            request: impl tonic::IntoRequest<super::MergeBranchRequest>,
        ) -> std::result::Result<
            tonic::Response<super::MergeBranchResponse>,
            tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::new(
                        tonic::Code::Unknown,
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic::codec::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/MergeBranch",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "trogonatlas.api.eventmodel.v1alpha1.EventModelService",
                        "MergeBranch",
                    ),
                );
            self.inner.unary(req, path, codec).await
        }
        ///
        pub async fn update_branch(
            &mut self,
            request: impl tonic::IntoRequest<super::UpdateBranchRequest>,
        ) -> std::result::Result<
            tonic::Response<super::UpdateBranchResponse>,
            tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::new(
                        tonic::Code::Unknown,
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic::codec::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/UpdateBranch",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "trogonatlas.api.eventmodel.v1alpha1.EventModelService",
                        "UpdateBranch",
                    ),
                );
            self.inner.unary(req, path, codec).await
        }
        ///
        pub async fn resolve_branch_entry(
            &mut self,
            request: impl tonic::IntoRequest<super::ResolveBranchEntryRequest>,
        ) -> std::result::Result<
            tonic::Response<super::ResolveBranchEntryResponse>,
            tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::new(
                        tonic::Code::Unknown,
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic::codec::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/ResolveBranchEntry",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "trogonatlas.api.eventmodel.v1alpha1.EventModelService",
                        "ResolveBranchEntry",
                    ),
                );
            self.inner.unary(req, path, codec).await
        }
        /** Tenant type libraries, compiled from their stored source on demand.
*/
        pub async fn compile_type_library(
            &mut self,
            request: impl tonic::IntoRequest<super::CompileTypeLibraryRequest>,
        ) -> std::result::Result<
            tonic::Response<super::CompileTypeLibraryResponse>,
            tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::new(
                        tonic::Code::Unknown,
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic::codec::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/CompileTypeLibrary",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "trogonatlas.api.eventmodel.v1alpha1.EventModelService",
                        "CompileTypeLibrary",
                    ),
                );
            self.inner.unary(req, path, codec).await
        }
        ///
        pub async fn resolve_type(
            &mut self,
            request: impl tonic::IntoRequest<super::ResolveTypeRequest>,
        ) -> std::result::Result<
            tonic::Response<super::ResolveTypeResponse>,
            tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::new(
                        tonic::Code::Unknown,
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic::codec::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/ResolveType",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "trogonatlas.api.eventmodel.v1alpha1.EventModelService",
                        "ResolveType",
                    ),
                );
            self.inner.unary(req, path, codec).await
        }
        ///
        pub async fn get_type_library_descriptor_set(
            &mut self,
            request: impl tonic::IntoRequest<super::GetTypeLibraryDescriptorSetRequest>,
        ) -> std::result::Result<
            tonic::Response<super::GetTypeLibraryDescriptorSetResponse>,
            tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::new(
                        tonic::Code::Unknown,
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic::codec::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/GetTypeLibraryDescriptorSet",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "trogonatlas.api.eventmodel.v1alpha1.EventModelService",
                        "GetTypeLibraryDescriptorSet",
                    ),
                );
            self.inner.unary(req, path, codec).await
        }
    }
}
/// Generated server implementations.
pub mod event_model_service_server {
    #![allow(unused_variables, dead_code, missing_docs, clippy::let_unit_value)]
    use tonic::codegen::*;
    /// Generated trait containing gRPC methods that should be implemented for use with EventModelServiceServer.
    #[async_trait]
    pub trait EventModelService: Send + Sync + 'static {
        /** Bootstrap. Clients SHOULD call GetServerInfo on connect to learn
 which optional features and limits apply (Decision #22).
*/
        async fn get_server_info(
            &self,
            request: tonic::Request<super::GetServerInfoRequest>,
        ) -> std::result::Result<
            tonic::Response<super::GetServerInfoResponse>,
            tonic::Status,
        >;
        ///
        async fn list_namespaces(
            &self,
            request: tonic::Request<super::ListNamespacesRequest>,
        ) -> std::result::Result<
            tonic::Response<super::ListNamespacesResponse>,
            tonic::Status,
        >;
        /** Caller identity. Reader-level: any authenticated principal may learn
 its own role, namespaces, and owner boundary.
*/
        async fn who_am_i(
            &self,
            request: tonic::Request<super::WhoAmIRequest>,
        ) -> std::result::Result<tonic::Response<super::WhoAmIResponse>, tonic::Status>;
        /** Namespace registry. Registering claims a human name under an ownership
 node and mints the immutable id that storage keys use; moving re-parents
 an existing namespace without touching a single entity.
*/
        async fn register_namespace(
            &self,
            request: tonic::Request<super::RegisterNamespaceRequest>,
        ) -> std::result::Result<
            tonic::Response<super::RegisterNamespaceResponse>,
            tonic::Status,
        >;
        ///
        async fn move_namespace(
            &self,
            request: tonic::Request<super::MoveNamespaceRequest>,
        ) -> std::result::Result<
            tonic::Response<super::MoveNamespaceResponse>,
            tonic::Status,
        >;
        /** Content-addressable identity of the current state. Branch-scoped via
 the `x-trogon-atlas-branch` header like every other read.
*/
        async fn get_snapshot_id(
            &self,
            request: tonic::Request<super::GetSnapshotIdRequest>,
        ) -> std::result::Result<
            tonic::Response<super::GetSnapshotIdResponse>,
            tonic::Status,
        >;
        /** Direct lookup.
*/
        async fn get_entity(
            &self,
            request: tonic::Request<super::GetEntityRequest>,
        ) -> std::result::Result<
            tonic::Response<super::GetEntityResponse>,
            tonic::Status,
        >;
        ///
        async fn batch_get_entities(
            &self,
            request: tonic::Request<super::BatchGetEntitiesRequest>,
        ) -> std::result::Result<
            tonic::Response<super::BatchGetEntitiesResponse>,
            tonic::Status,
        >;
        /** Discovery.
*/
        async fn list_entities(
            &self,
            request: tonic::Request<super::ListEntitiesRequest>,
        ) -> std::result::Result<
            tonic::Response<super::ListEntitiesResponse>,
            tonic::Status,
        >;
        ///
        async fn search_entities(
            &self,
            request: tonic::Request<super::SearchEntitiesRequest>,
        ) -> std::result::Result<
            tonic::Response<super::SearchEntitiesResponse>,
            tonic::Status,
        >;
        /** Versioning.
*/
        async fn list_versions(
            &self,
            request: tonic::Request<super::ListVersionsRequest>,
        ) -> std::result::Result<
            tonic::Response<super::ListVersionsResponse>,
            tonic::Status,
        >;
        ///
        async fn get_latest_version(
            &self,
            request: tonic::Request<super::GetLatestVersionRequest>,
        ) -> std::result::Result<
            tonic::Response<super::GetLatestVersionResponse>,
            tonic::Status,
        >;
        ///
        async fn get_supersession_chain(
            &self,
            request: tonic::Request<super::GetSupersessionChainRequest>,
        ) -> std::result::Result<
            tonic::Response<super::GetSupersessionChainResponse>,
            tonic::Status,
        >;
        /** Graph traversal across typed Refs/Edges.
*/
        async fn get_incoming_references(
            &self,
            request: tonic::Request<super::GetReferencesRequest>,
        ) -> std::result::Result<
            tonic::Response<super::GetReferencesResponse>,
            tonic::Status,
        >;
        ///
        async fn get_outgoing_references(
            &self,
            request: tonic::Request<super::GetReferencesRequest>,
        ) -> std::result::Result<
            tonic::Response<super::GetReferencesResponse>,
            tonic::Status,
        >;
        /** Transitive reverse-reference closure: "what depends on this?"
*/
        async fn get_impact(
            &self,
            request: tonic::Request<super::GetImpactRequest>,
        ) -> std::result::Result<
            tonic::Response<super::GetImpactResponse>,
            tonic::Status,
        >;
        /** Rewrite every stored reference to `from` so it points at `to`. Used as
 the rewire step in the promotion workflow: create v+1 with supersedes,
 call retarget_references (dry_run first to preview), then drain any
 residual SLICE_STALE_REF findings. For partial migrations or when only
 a subset of referrers should move, fall back to get_incoming_references
 + batch_mutate.
*/
        async fn retarget_references(
            &self,
            request: tonic::Request<super::RetargetReferencesRequest>,
        ) -> std::result::Result<
            tonic::Response<super::RetargetReferencesResponse>,
            tonic::Status,
        >;
        /** Denormalized projections. These are the primary RPCs for MCP tools.
*/
        async fn get_slice_projection(
            &self,
            request: tonic::Request<super::GetSliceProjectionRequest>,
        ) -> std::result::Result<
            tonic::Response<super::GetSliceProjectionResponse>,
            tonic::Status,
        >;
        ///
        async fn get_storyboard_projection(
            &self,
            request: tonic::Request<super::GetStoryboardProjectionRequest>,
        ) -> std::result::Result<
            tonic::Response<super::GetStoryboardProjectionResponse>,
            tonic::Status,
        >;
        ///
        async fn get_event_model_projection(
            &self,
            request: tonic::Request<super::GetEventModelProjectionRequest>,
        ) -> std::result::Result<
            tonic::Response<super::GetEventModelProjectionResponse>,
            tonic::Status,
        >;
        /** Closure-as-new-model: extract a slice/storyboard's reachable graph
 into a fresh, self-contained EventModel.
*/
        async fn extract_subgraph(
            &self,
            request: tonic::Request<super::ExtractSubgraphRequest>,
        ) -> std::result::Result<
            tonic::Response<super::ExtractSubgraphResponse>,
            tonic::Status,
        >;
        /** Structural comparison of two entities (typically two versions).
*/
        async fn diff_entities(
            &self,
            request: tonic::Request<super::DiffEntitiesRequest>,
        ) -> std::result::Result<
            tonic::Response<super::DiffEntitiesResponse>,
            tonic::Status,
        >;
        /** Mutation. PutEntity / DeleteEntity are single-entity. BatchMutate is
 atomic across an ordered list of ops (Decision #23).
*/
        async fn put_entity(
            &self,
            request: tonic::Request<super::PutEntityRequest>,
        ) -> std::result::Result<
            tonic::Response<super::PutEntityResponse>,
            tonic::Status,
        >;
        ///
        async fn delete_entity(
            &self,
            request: tonic::Request<super::DeleteEntityRequest>,
        ) -> std::result::Result<
            tonic::Response<super::DeleteEntityResponse>,
            tonic::Status,
        >;
        ///
        async fn batch_mutate(
            &self,
            request: tonic::Request<super::BatchMutateRequest>,
        ) -> std::result::Result<
            tonic::Response<super::BatchMutateResponse>,
            tonic::Status,
        >;
        /** Poll-based change feed (Decision #19: unary, not streaming).
*/
        async fn list_changes(
            &self,
            request: tonic::Request<super::ListChangesRequest>,
        ) -> std::result::Result<
            tonic::Response<super::ListChangesResponse>,
            tonic::Status,
        >;
        /// Server streaming response type for the StreamChanges method.
        type StreamChangesStream: tonic::codegen::tokio_stream::Stream<
                Item = std::result::Result<
                    super::super::super::super::catalog::v1alpha1::ChangeEvent,
                    tonic::Status,
                >,
            >
            + Send
            + 'static;
        /** Server-streaming change feed for non-MCP consumers. Same data as
 ListChanges, push semantics. MCP adapters should prefer ListChanges.
*/
        async fn stream_changes(
            &self,
            request: tonic::Request<super::StreamChangesRequest>,
        ) -> std::result::Result<
            tonic::Response<Self::StreamChangesStream>,
            tonic::Status,
        >;
        /** Durable changeset log: what landed together, who did it, when. Unlike
 the change feed these records do not expire and are not dropped when a
 publish fails.
*/
        async fn list_changesets(
            &self,
            request: tonic::Request<super::ListChangesetsRequest>,
        ) -> std::result::Result<
            tonic::Response<super::ListChangesetsResponse>,
            tonic::Status,
        >;
        ///
        async fn get_changeset(
            &self,
            request: tonic::Request<super::GetChangesetRequest>,
        ) -> std::result::Result<
            tonic::Response<super::GetChangesetResponse>,
            tonic::Status,
        >;
        /** Per-entity edit log and undo. See docs/explanation/changesets.md.
*/
        async fn get_entity_history(
            &self,
            request: tonic::Request<super::GetEntityHistoryRequest>,
        ) -> std::result::Result<
            tonic::Response<super::GetEntityHistoryResponse>,
            tonic::Status,
        >;
        ///
        async fn revert_changeset(
            &self,
            request: tonic::Request<super::RevertChangesetRequest>,
        ) -> std::result::Result<
            tonic::Response<super::RevertChangesetResponse>,
            tonic::Status,
        >;
        /** Look up the receipt for an operation_id this caller claimed on a prior
 mutation call. Scoped to the calling principal.
*/
        async fn get_operation(
            &self,
            request: tonic::Request<super::GetOperationRequest>,
        ) -> std::result::Result<
            tonic::Response<super::GetOperationResponse>,
            tonic::Status,
        >;
        /** Validation.
*/
        async fn validate_event_model(
            &self,
            request: tonic::Request<super::ValidateEventModelRequest>,
        ) -> std::result::Result<
            tonic::Response<super::ValidateEventModelResponse>,
            tonic::Status,
        >;
        /** Returns the static catalog of every rule the validator can emit.
 Stable across runs; the only thing that changes is the codebase
 version. Use this to drive UI legends, doc links, and CI severity
 gates.
*/
        async fn list_validation_rules(
            &self,
            request: tonic::Request<super::ListValidationRulesRequest>,
        ) -> std::result::Result<
            tonic::Response<super::ListValidationRulesResponse>,
            tonic::Status,
        >;
        /** Validate every EventModel in a project (or domain) in one call.
*/
        async fn validate_project(
            &self,
            request: tonic::Request<super::ValidateProjectRequest>,
        ) -> std::result::Result<
            tonic::Response<super::ValidateProjectResponse>,
            tonic::Status,
        >;
        /** Bulk delete by structural query (project/namespace/kind/slug). One
 call replaces the "list-then-batch" client-side pattern.
*/
        async fn delete_by_query(
            &self,
            request: tonic::Request<super::DeleteByQueryRequest>,
        ) -> std::result::Result<
            tonic::Response<super::DeleteByQueryResponse>,
            tonic::Status,
        >;
        /** Static catalog of every EntityKind the system stores.
*/
        async fn list_entity_kinds(
            &self,
            request: tonic::Request<super::ListEntityKindsRequest>,
        ) -> std::result::Result<
            tonic::Response<super::ListEntityKindsResponse>,
            tonic::Status,
        >;
        /** List every entity scoped to a Domain (walks the EM->BC->Subdomain->Domain
 realizes chain server-side so clients don't have to).
*/
        async fn list_entities_by_domain(
            &self,
            request: tonic::Request<super::ListEntitiesByDomainRequest>,
        ) -> std::result::Result<
            tonic::Response<super::ListEntitiesByDomainResponse>,
            tonic::Status,
        >;
        /** Analysis. The AI does the heavy lifting; humans review the gaps it
 surfaces. Neither RPC mutates the model.
*/
        async fn infer_data_flow(
            &self,
            request: tonic::Request<super::InferDataFlowRequest>,
        ) -> std::result::Result<
            tonic::Response<super::InferDataFlowResponse>,
            tonic::Status,
        >;
        ///
        async fn check_information_completeness(
            &self,
            request: tonic::Request<super::CheckInformationCompletenessRequest>,
        ) -> std::result::Result<
            tonic::Response<super::CheckInformationCompletenessResponse>,
            tonic::Status,
        >;
        /** Branch lifecycle (Phase 1: Isolation). See the "Branch context" note
 near the top of this file for how every other RPC picks up branch scope.
*/
        async fn create_branch(
            &self,
            request: tonic::Request<super::CreateBranchRequest>,
        ) -> std::result::Result<
            tonic::Response<super::CreateBranchResponse>,
            tonic::Status,
        >;
        ///
        async fn list_branches(
            &self,
            request: tonic::Request<super::ListBranchesRequest>,
        ) -> std::result::Result<
            tonic::Response<super::ListBranchesResponse>,
            tonic::Status,
        >;
        ///
        async fn delete_branch(
            &self,
            request: tonic::Request<super::DeleteBranchRequest>,
        ) -> std::result::Result<
            tonic::Response<super::DeleteBranchResponse>,
            tonic::Status,
        >;
        /** Branch review and merge (Phase 2). `name` is an explicit request field,
 like the three lifecycle RPCs above, since these operate ON a named
 branch rather than being scoped BY a branch context.
*/
        async fn diff_branch(
            &self,
            request: tonic::Request<super::DiffBranchRequest>,
        ) -> std::result::Result<
            tonic::Response<super::DiffBranchResponse>,
            tonic::Status,
        >;
        ///
        async fn merge_branch(
            &self,
            request: tonic::Request<super::MergeBranchRequest>,
        ) -> std::result::Result<
            tonic::Response<super::MergeBranchResponse>,
            tonic::Status,
        >;
        ///
        async fn update_branch(
            &self,
            request: tonic::Request<super::UpdateBranchRequest>,
        ) -> std::result::Result<
            tonic::Response<super::UpdateBranchResponse>,
            tonic::Status,
        >;
        ///
        async fn resolve_branch_entry(
            &self,
            request: tonic::Request<super::ResolveBranchEntryRequest>,
        ) -> std::result::Result<
            tonic::Response<super::ResolveBranchEntryResponse>,
            tonic::Status,
        >;
        /** Tenant type libraries, compiled from their stored source on demand.
*/
        async fn compile_type_library(
            &self,
            request: tonic::Request<super::CompileTypeLibraryRequest>,
        ) -> std::result::Result<
            tonic::Response<super::CompileTypeLibraryResponse>,
            tonic::Status,
        >;
        ///
        async fn resolve_type(
            &self,
            request: tonic::Request<super::ResolveTypeRequest>,
        ) -> std::result::Result<
            tonic::Response<super::ResolveTypeResponse>,
            tonic::Status,
        >;
        ///
        async fn get_type_library_descriptor_set(
            &self,
            request: tonic::Request<super::GetTypeLibraryDescriptorSetRequest>,
        ) -> std::result::Result<
            tonic::Response<super::GetTypeLibraryDescriptorSetResponse>,
            tonic::Status,
        >;
    }
    ///
    #[derive(Debug)]
    pub struct EventModelServiceServer<T: EventModelService> {
        inner: Arc<T>,
        accept_compression_encodings: EnabledCompressionEncodings,
        send_compression_encodings: EnabledCompressionEncodings,
        max_decoding_message_size: Option<usize>,
        max_encoding_message_size: Option<usize>,
    }
    impl<T: EventModelService> EventModelServiceServer<T> {
        pub fn new(inner: T) -> Self {
            Self::from_arc(Arc::new(inner))
        }
        pub fn from_arc(inner: Arc<T>) -> Self {
            Self {
                inner,
                accept_compression_encodings: Default::default(),
                send_compression_encodings: Default::default(),
                max_decoding_message_size: None,
                max_encoding_message_size: None,
            }
        }
        pub fn with_interceptor<F>(
            inner: T,
            interceptor: F,
        ) -> InterceptedService<Self, F>
        where
            F: tonic::service::Interceptor,
        {
            InterceptedService::new(Self::new(inner), interceptor)
        }
        /// Enable decompressing requests with the given encoding.
        #[must_use]
        pub fn accept_compressed(mut self, encoding: CompressionEncoding) -> Self {
            self.accept_compression_encodings.enable(encoding);
            self
        }
        /// Compress responses with the given encoding, if the client supports it.
        #[must_use]
        pub fn send_compressed(mut self, encoding: CompressionEncoding) -> Self {
            self.send_compression_encodings.enable(encoding);
            self
        }
        /// Limits the maximum size of a decoded message.
        ///
        /// Default: `4MB`
        #[must_use]
        pub fn max_decoding_message_size(mut self, limit: usize) -> Self {
            self.max_decoding_message_size = Some(limit);
            self
        }
        /// Limits the maximum size of an encoded message.
        ///
        /// Default: `usize::MAX`
        #[must_use]
        pub fn max_encoding_message_size(mut self, limit: usize) -> Self {
            self.max_encoding_message_size = Some(limit);
            self
        }
    }
    impl<T, B> tonic::codegen::Service<http::Request<B>> for EventModelServiceServer<T>
    where
        T: EventModelService,
        B: Body + Send + 'static,
        B::Error: Into<StdError> + Send + 'static,
    {
        type Response = http::Response<tonic::body::BoxBody>;
        type Error = std::convert::Infallible;
        type Future = BoxFuture<Self::Response, Self::Error>;
        fn poll_ready(
            &mut self,
            _cx: &mut Context<'_>,
        ) -> Poll<std::result::Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }
        fn call(&mut self, req: http::Request<B>) -> Self::Future {
            match req.uri().path() {
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/GetServerInfo" => {
                    #[allow(non_camel_case_types)]
                    struct GetServerInfoSvc<T: EventModelService>(pub Arc<T>);
                    impl<
                        T: EventModelService,
                    > tonic::server::UnaryService<super::GetServerInfoRequest>
                    for GetServerInfoSvc<T> {
                        type Response = super::GetServerInfoResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::GetServerInfoRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as EventModelService>::get_server_info(&inner, request)
                                    .await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = GetServerInfoSvc(inner);
                        let codec = tonic::codec::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/ListNamespaces" => {
                    #[allow(non_camel_case_types)]
                    struct ListNamespacesSvc<T: EventModelService>(pub Arc<T>);
                    impl<
                        T: EventModelService,
                    > tonic::server::UnaryService<super::ListNamespacesRequest>
                    for ListNamespacesSvc<T> {
                        type Response = super::ListNamespacesResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::ListNamespacesRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as EventModelService>::list_namespaces(&inner, request)
                                    .await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = ListNamespacesSvc(inner);
                        let codec = tonic::codec::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/WhoAmI" => {
                    #[allow(non_camel_case_types)]
                    struct WhoAmISvc<T: EventModelService>(pub Arc<T>);
                    impl<
                        T: EventModelService,
                    > tonic::server::UnaryService<super::WhoAmIRequest>
                    for WhoAmISvc<T> {
                        type Response = super::WhoAmIResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::WhoAmIRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as EventModelService>::who_am_i(&inner, request).await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = WhoAmISvc(inner);
                        let codec = tonic::codec::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/RegisterNamespace" => {
                    #[allow(non_camel_case_types)]
                    struct RegisterNamespaceSvc<T: EventModelService>(pub Arc<T>);
                    impl<
                        T: EventModelService,
                    > tonic::server::UnaryService<super::RegisterNamespaceRequest>
                    for RegisterNamespaceSvc<T> {
                        type Response = super::RegisterNamespaceResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::RegisterNamespaceRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as EventModelService>::register_namespace(
                                        &inner,
                                        request,
                                    )
                                    .await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = RegisterNamespaceSvc(inner);
                        let codec = tonic::codec::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/MoveNamespace" => {
                    #[allow(non_camel_case_types)]
                    struct MoveNamespaceSvc<T: EventModelService>(pub Arc<T>);
                    impl<
                        T: EventModelService,
                    > tonic::server::UnaryService<super::MoveNamespaceRequest>
                    for MoveNamespaceSvc<T> {
                        type Response = super::MoveNamespaceResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::MoveNamespaceRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as EventModelService>::move_namespace(&inner, request)
                                    .await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = MoveNamespaceSvc(inner);
                        let codec = tonic::codec::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/GetSnapshotId" => {
                    #[allow(non_camel_case_types)]
                    struct GetSnapshotIdSvc<T: EventModelService>(pub Arc<T>);
                    impl<
                        T: EventModelService,
                    > tonic::server::UnaryService<super::GetSnapshotIdRequest>
                    for GetSnapshotIdSvc<T> {
                        type Response = super::GetSnapshotIdResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::GetSnapshotIdRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as EventModelService>::get_snapshot_id(&inner, request)
                                    .await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = GetSnapshotIdSvc(inner);
                        let codec = tonic::codec::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/GetEntity" => {
                    #[allow(non_camel_case_types)]
                    struct GetEntitySvc<T: EventModelService>(pub Arc<T>);
                    impl<
                        T: EventModelService,
                    > tonic::server::UnaryService<super::GetEntityRequest>
                    for GetEntitySvc<T> {
                        type Response = super::GetEntityResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::GetEntityRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as EventModelService>::get_entity(&inner, request).await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = GetEntitySvc(inner);
                        let codec = tonic::codec::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/BatchGetEntities" => {
                    #[allow(non_camel_case_types)]
                    struct BatchGetEntitiesSvc<T: EventModelService>(pub Arc<T>);
                    impl<
                        T: EventModelService,
                    > tonic::server::UnaryService<super::BatchGetEntitiesRequest>
                    for BatchGetEntitiesSvc<T> {
                        type Response = super::BatchGetEntitiesResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::BatchGetEntitiesRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as EventModelService>::batch_get_entities(
                                        &inner,
                                        request,
                                    )
                                    .await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = BatchGetEntitiesSvc(inner);
                        let codec = tonic::codec::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/ListEntities" => {
                    #[allow(non_camel_case_types)]
                    struct ListEntitiesSvc<T: EventModelService>(pub Arc<T>);
                    impl<
                        T: EventModelService,
                    > tonic::server::UnaryService<super::ListEntitiesRequest>
                    for ListEntitiesSvc<T> {
                        type Response = super::ListEntitiesResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::ListEntitiesRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as EventModelService>::list_entities(&inner, request)
                                    .await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = ListEntitiesSvc(inner);
                        let codec = tonic::codec::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/SearchEntities" => {
                    #[allow(non_camel_case_types)]
                    struct SearchEntitiesSvc<T: EventModelService>(pub Arc<T>);
                    impl<
                        T: EventModelService,
                    > tonic::server::UnaryService<super::SearchEntitiesRequest>
                    for SearchEntitiesSvc<T> {
                        type Response = super::SearchEntitiesResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::SearchEntitiesRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as EventModelService>::search_entities(&inner, request)
                                    .await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = SearchEntitiesSvc(inner);
                        let codec = tonic::codec::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/ListVersions" => {
                    #[allow(non_camel_case_types)]
                    struct ListVersionsSvc<T: EventModelService>(pub Arc<T>);
                    impl<
                        T: EventModelService,
                    > tonic::server::UnaryService<super::ListVersionsRequest>
                    for ListVersionsSvc<T> {
                        type Response = super::ListVersionsResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::ListVersionsRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as EventModelService>::list_versions(&inner, request)
                                    .await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = ListVersionsSvc(inner);
                        let codec = tonic::codec::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/GetLatestVersion" => {
                    #[allow(non_camel_case_types)]
                    struct GetLatestVersionSvc<T: EventModelService>(pub Arc<T>);
                    impl<
                        T: EventModelService,
                    > tonic::server::UnaryService<super::GetLatestVersionRequest>
                    for GetLatestVersionSvc<T> {
                        type Response = super::GetLatestVersionResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::GetLatestVersionRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as EventModelService>::get_latest_version(
                                        &inner,
                                        request,
                                    )
                                    .await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = GetLatestVersionSvc(inner);
                        let codec = tonic::codec::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/GetSupersessionChain" => {
                    #[allow(non_camel_case_types)]
                    struct GetSupersessionChainSvc<T: EventModelService>(pub Arc<T>);
                    impl<
                        T: EventModelService,
                    > tonic::server::UnaryService<super::GetSupersessionChainRequest>
                    for GetSupersessionChainSvc<T> {
                        type Response = super::GetSupersessionChainResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::GetSupersessionChainRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as EventModelService>::get_supersession_chain(
                                        &inner,
                                        request,
                                    )
                                    .await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = GetSupersessionChainSvc(inner);
                        let codec = tonic::codec::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/GetIncomingReferences" => {
                    #[allow(non_camel_case_types)]
                    struct GetIncomingReferencesSvc<T: EventModelService>(pub Arc<T>);
                    impl<
                        T: EventModelService,
                    > tonic::server::UnaryService<super::GetReferencesRequest>
                    for GetIncomingReferencesSvc<T> {
                        type Response = super::GetReferencesResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::GetReferencesRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as EventModelService>::get_incoming_references(
                                        &inner,
                                        request,
                                    )
                                    .await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = GetIncomingReferencesSvc(inner);
                        let codec = tonic::codec::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/GetOutgoingReferences" => {
                    #[allow(non_camel_case_types)]
                    struct GetOutgoingReferencesSvc<T: EventModelService>(pub Arc<T>);
                    impl<
                        T: EventModelService,
                    > tonic::server::UnaryService<super::GetReferencesRequest>
                    for GetOutgoingReferencesSvc<T> {
                        type Response = super::GetReferencesResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::GetReferencesRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as EventModelService>::get_outgoing_references(
                                        &inner,
                                        request,
                                    )
                                    .await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = GetOutgoingReferencesSvc(inner);
                        let codec = tonic::codec::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/GetImpact" => {
                    #[allow(non_camel_case_types)]
                    struct GetImpactSvc<T: EventModelService>(pub Arc<T>);
                    impl<
                        T: EventModelService,
                    > tonic::server::UnaryService<super::GetImpactRequest>
                    for GetImpactSvc<T> {
                        type Response = super::GetImpactResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::GetImpactRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as EventModelService>::get_impact(&inner, request).await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = GetImpactSvc(inner);
                        let codec = tonic::codec::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/RetargetReferences" => {
                    #[allow(non_camel_case_types)]
                    struct RetargetReferencesSvc<T: EventModelService>(pub Arc<T>);
                    impl<
                        T: EventModelService,
                    > tonic::server::UnaryService<super::RetargetReferencesRequest>
                    for RetargetReferencesSvc<T> {
                        type Response = super::RetargetReferencesResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::RetargetReferencesRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as EventModelService>::retarget_references(
                                        &inner,
                                        request,
                                    )
                                    .await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = RetargetReferencesSvc(inner);
                        let codec = tonic::codec::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/GetSliceProjection" => {
                    #[allow(non_camel_case_types)]
                    struct GetSliceProjectionSvc<T: EventModelService>(pub Arc<T>);
                    impl<
                        T: EventModelService,
                    > tonic::server::UnaryService<super::GetSliceProjectionRequest>
                    for GetSliceProjectionSvc<T> {
                        type Response = super::GetSliceProjectionResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::GetSliceProjectionRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as EventModelService>::get_slice_projection(
                                        &inner,
                                        request,
                                    )
                                    .await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = GetSliceProjectionSvc(inner);
                        let codec = tonic::codec::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/GetStoryboardProjection" => {
                    #[allow(non_camel_case_types)]
                    struct GetStoryboardProjectionSvc<T: EventModelService>(pub Arc<T>);
                    impl<
                        T: EventModelService,
                    > tonic::server::UnaryService<super::GetStoryboardProjectionRequest>
                    for GetStoryboardProjectionSvc<T> {
                        type Response = super::GetStoryboardProjectionResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<
                                super::GetStoryboardProjectionRequest,
                            >,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as EventModelService>::get_storyboard_projection(
                                        &inner,
                                        request,
                                    )
                                    .await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = GetStoryboardProjectionSvc(inner);
                        let codec = tonic::codec::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/GetEventModelProjection" => {
                    #[allow(non_camel_case_types)]
                    struct GetEventModelProjectionSvc<T: EventModelService>(pub Arc<T>);
                    impl<
                        T: EventModelService,
                    > tonic::server::UnaryService<super::GetEventModelProjectionRequest>
                    for GetEventModelProjectionSvc<T> {
                        type Response = super::GetEventModelProjectionResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<
                                super::GetEventModelProjectionRequest,
                            >,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as EventModelService>::get_event_model_projection(
                                        &inner,
                                        request,
                                    )
                                    .await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = GetEventModelProjectionSvc(inner);
                        let codec = tonic::codec::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/ExtractSubgraph" => {
                    #[allow(non_camel_case_types)]
                    struct ExtractSubgraphSvc<T: EventModelService>(pub Arc<T>);
                    impl<
                        T: EventModelService,
                    > tonic::server::UnaryService<super::ExtractSubgraphRequest>
                    for ExtractSubgraphSvc<T> {
                        type Response = super::ExtractSubgraphResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::ExtractSubgraphRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as EventModelService>::extract_subgraph(&inner, request)
                                    .await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = ExtractSubgraphSvc(inner);
                        let codec = tonic::codec::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/DiffEntities" => {
                    #[allow(non_camel_case_types)]
                    struct DiffEntitiesSvc<T: EventModelService>(pub Arc<T>);
                    impl<
                        T: EventModelService,
                    > tonic::server::UnaryService<super::DiffEntitiesRequest>
                    for DiffEntitiesSvc<T> {
                        type Response = super::DiffEntitiesResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::DiffEntitiesRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as EventModelService>::diff_entities(&inner, request)
                                    .await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = DiffEntitiesSvc(inner);
                        let codec = tonic::codec::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/PutEntity" => {
                    #[allow(non_camel_case_types)]
                    struct PutEntitySvc<T: EventModelService>(pub Arc<T>);
                    impl<
                        T: EventModelService,
                    > tonic::server::UnaryService<super::PutEntityRequest>
                    for PutEntitySvc<T> {
                        type Response = super::PutEntityResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::PutEntityRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as EventModelService>::put_entity(&inner, request).await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = PutEntitySvc(inner);
                        let codec = tonic::codec::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/DeleteEntity" => {
                    #[allow(non_camel_case_types)]
                    struct DeleteEntitySvc<T: EventModelService>(pub Arc<T>);
                    impl<
                        T: EventModelService,
                    > tonic::server::UnaryService<super::DeleteEntityRequest>
                    for DeleteEntitySvc<T> {
                        type Response = super::DeleteEntityResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::DeleteEntityRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as EventModelService>::delete_entity(&inner, request)
                                    .await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = DeleteEntitySvc(inner);
                        let codec = tonic::codec::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/BatchMutate" => {
                    #[allow(non_camel_case_types)]
                    struct BatchMutateSvc<T: EventModelService>(pub Arc<T>);
                    impl<
                        T: EventModelService,
                    > tonic::server::UnaryService<super::BatchMutateRequest>
                    for BatchMutateSvc<T> {
                        type Response = super::BatchMutateResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::BatchMutateRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as EventModelService>::batch_mutate(&inner, request)
                                    .await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = BatchMutateSvc(inner);
                        let codec = tonic::codec::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/ListChanges" => {
                    #[allow(non_camel_case_types)]
                    struct ListChangesSvc<T: EventModelService>(pub Arc<T>);
                    impl<
                        T: EventModelService,
                    > tonic::server::UnaryService<super::ListChangesRequest>
                    for ListChangesSvc<T> {
                        type Response = super::ListChangesResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::ListChangesRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as EventModelService>::list_changes(&inner, request)
                                    .await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = ListChangesSvc(inner);
                        let codec = tonic::codec::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/StreamChanges" => {
                    #[allow(non_camel_case_types)]
                    struct StreamChangesSvc<T: EventModelService>(pub Arc<T>);
                    impl<
                        T: EventModelService,
                    > tonic::server::ServerStreamingService<super::StreamChangesRequest>
                    for StreamChangesSvc<T> {
                        type Response = super::super::super::super::catalog::v1alpha1::ChangeEvent;
                        type ResponseStream = T::StreamChangesStream;
                        type Future = BoxFuture<
                            tonic::Response<Self::ResponseStream>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::StreamChangesRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as EventModelService>::stream_changes(&inner, request)
                                    .await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = StreamChangesSvc(inner);
                        let codec = tonic::codec::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.server_streaming(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/ListChangesets" => {
                    #[allow(non_camel_case_types)]
                    struct ListChangesetsSvc<T: EventModelService>(pub Arc<T>);
                    impl<
                        T: EventModelService,
                    > tonic::server::UnaryService<super::ListChangesetsRequest>
                    for ListChangesetsSvc<T> {
                        type Response = super::ListChangesetsResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::ListChangesetsRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as EventModelService>::list_changesets(&inner, request)
                                    .await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = ListChangesetsSvc(inner);
                        let codec = tonic::codec::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/GetChangeset" => {
                    #[allow(non_camel_case_types)]
                    struct GetChangesetSvc<T: EventModelService>(pub Arc<T>);
                    impl<
                        T: EventModelService,
                    > tonic::server::UnaryService<super::GetChangesetRequest>
                    for GetChangesetSvc<T> {
                        type Response = super::GetChangesetResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::GetChangesetRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as EventModelService>::get_changeset(&inner, request)
                                    .await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = GetChangesetSvc(inner);
                        let codec = tonic::codec::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/GetEntityHistory" => {
                    #[allow(non_camel_case_types)]
                    struct GetEntityHistorySvc<T: EventModelService>(pub Arc<T>);
                    impl<
                        T: EventModelService,
                    > tonic::server::UnaryService<super::GetEntityHistoryRequest>
                    for GetEntityHistorySvc<T> {
                        type Response = super::GetEntityHistoryResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::GetEntityHistoryRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as EventModelService>::get_entity_history(
                                        &inner,
                                        request,
                                    )
                                    .await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = GetEntityHistorySvc(inner);
                        let codec = tonic::codec::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/RevertChangeset" => {
                    #[allow(non_camel_case_types)]
                    struct RevertChangesetSvc<T: EventModelService>(pub Arc<T>);
                    impl<
                        T: EventModelService,
                    > tonic::server::UnaryService<super::RevertChangesetRequest>
                    for RevertChangesetSvc<T> {
                        type Response = super::RevertChangesetResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::RevertChangesetRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as EventModelService>::revert_changeset(&inner, request)
                                    .await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = RevertChangesetSvc(inner);
                        let codec = tonic::codec::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/GetOperation" => {
                    #[allow(non_camel_case_types)]
                    struct GetOperationSvc<T: EventModelService>(pub Arc<T>);
                    impl<
                        T: EventModelService,
                    > tonic::server::UnaryService<super::GetOperationRequest>
                    for GetOperationSvc<T> {
                        type Response = super::GetOperationResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::GetOperationRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as EventModelService>::get_operation(&inner, request)
                                    .await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = GetOperationSvc(inner);
                        let codec = tonic::codec::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/ValidateEventModel" => {
                    #[allow(non_camel_case_types)]
                    struct ValidateEventModelSvc<T: EventModelService>(pub Arc<T>);
                    impl<
                        T: EventModelService,
                    > tonic::server::UnaryService<super::ValidateEventModelRequest>
                    for ValidateEventModelSvc<T> {
                        type Response = super::ValidateEventModelResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::ValidateEventModelRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as EventModelService>::validate_event_model(
                                        &inner,
                                        request,
                                    )
                                    .await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = ValidateEventModelSvc(inner);
                        let codec = tonic::codec::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/ListValidationRules" => {
                    #[allow(non_camel_case_types)]
                    struct ListValidationRulesSvc<T: EventModelService>(pub Arc<T>);
                    impl<
                        T: EventModelService,
                    > tonic::server::UnaryService<super::ListValidationRulesRequest>
                    for ListValidationRulesSvc<T> {
                        type Response = super::ListValidationRulesResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::ListValidationRulesRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as EventModelService>::list_validation_rules(
                                        &inner,
                                        request,
                                    )
                                    .await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = ListValidationRulesSvc(inner);
                        let codec = tonic::codec::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/ValidateProject" => {
                    #[allow(non_camel_case_types)]
                    struct ValidateProjectSvc<T: EventModelService>(pub Arc<T>);
                    impl<
                        T: EventModelService,
                    > tonic::server::UnaryService<super::ValidateProjectRequest>
                    for ValidateProjectSvc<T> {
                        type Response = super::ValidateProjectResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::ValidateProjectRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as EventModelService>::validate_project(&inner, request)
                                    .await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = ValidateProjectSvc(inner);
                        let codec = tonic::codec::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/DeleteByQuery" => {
                    #[allow(non_camel_case_types)]
                    struct DeleteByQuerySvc<T: EventModelService>(pub Arc<T>);
                    impl<
                        T: EventModelService,
                    > tonic::server::UnaryService<super::DeleteByQueryRequest>
                    for DeleteByQuerySvc<T> {
                        type Response = super::DeleteByQueryResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::DeleteByQueryRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as EventModelService>::delete_by_query(&inner, request)
                                    .await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = DeleteByQuerySvc(inner);
                        let codec = tonic::codec::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/ListEntityKinds" => {
                    #[allow(non_camel_case_types)]
                    struct ListEntityKindsSvc<T: EventModelService>(pub Arc<T>);
                    impl<
                        T: EventModelService,
                    > tonic::server::UnaryService<super::ListEntityKindsRequest>
                    for ListEntityKindsSvc<T> {
                        type Response = super::ListEntityKindsResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::ListEntityKindsRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as EventModelService>::list_entity_kinds(&inner, request)
                                    .await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = ListEntityKindsSvc(inner);
                        let codec = tonic::codec::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/ListEntitiesByDomain" => {
                    #[allow(non_camel_case_types)]
                    struct ListEntitiesByDomainSvc<T: EventModelService>(pub Arc<T>);
                    impl<
                        T: EventModelService,
                    > tonic::server::UnaryService<super::ListEntitiesByDomainRequest>
                    for ListEntitiesByDomainSvc<T> {
                        type Response = super::ListEntitiesByDomainResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::ListEntitiesByDomainRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as EventModelService>::list_entities_by_domain(
                                        &inner,
                                        request,
                                    )
                                    .await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = ListEntitiesByDomainSvc(inner);
                        let codec = tonic::codec::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/InferDataFlow" => {
                    #[allow(non_camel_case_types)]
                    struct InferDataFlowSvc<T: EventModelService>(pub Arc<T>);
                    impl<
                        T: EventModelService,
                    > tonic::server::UnaryService<super::InferDataFlowRequest>
                    for InferDataFlowSvc<T> {
                        type Response = super::InferDataFlowResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::InferDataFlowRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as EventModelService>::infer_data_flow(&inner, request)
                                    .await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = InferDataFlowSvc(inner);
                        let codec = tonic::codec::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/CheckInformationCompleteness" => {
                    #[allow(non_camel_case_types)]
                    struct CheckInformationCompletenessSvc<T: EventModelService>(
                        pub Arc<T>,
                    );
                    impl<
                        T: EventModelService,
                    > tonic::server::UnaryService<
                        super::CheckInformationCompletenessRequest,
                    > for CheckInformationCompletenessSvc<T> {
                        type Response = super::CheckInformationCompletenessResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<
                                super::CheckInformationCompletenessRequest,
                            >,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as EventModelService>::check_information_completeness(
                                        &inner,
                                        request,
                                    )
                                    .await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = CheckInformationCompletenessSvc(inner);
                        let codec = tonic::codec::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/CreateBranch" => {
                    #[allow(non_camel_case_types)]
                    struct CreateBranchSvc<T: EventModelService>(pub Arc<T>);
                    impl<
                        T: EventModelService,
                    > tonic::server::UnaryService<super::CreateBranchRequest>
                    for CreateBranchSvc<T> {
                        type Response = super::CreateBranchResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::CreateBranchRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as EventModelService>::create_branch(&inner, request)
                                    .await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = CreateBranchSvc(inner);
                        let codec = tonic::codec::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/ListBranches" => {
                    #[allow(non_camel_case_types)]
                    struct ListBranchesSvc<T: EventModelService>(pub Arc<T>);
                    impl<
                        T: EventModelService,
                    > tonic::server::UnaryService<super::ListBranchesRequest>
                    for ListBranchesSvc<T> {
                        type Response = super::ListBranchesResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::ListBranchesRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as EventModelService>::list_branches(&inner, request)
                                    .await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = ListBranchesSvc(inner);
                        let codec = tonic::codec::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/DeleteBranch" => {
                    #[allow(non_camel_case_types)]
                    struct DeleteBranchSvc<T: EventModelService>(pub Arc<T>);
                    impl<
                        T: EventModelService,
                    > tonic::server::UnaryService<super::DeleteBranchRequest>
                    for DeleteBranchSvc<T> {
                        type Response = super::DeleteBranchResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::DeleteBranchRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as EventModelService>::delete_branch(&inner, request)
                                    .await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = DeleteBranchSvc(inner);
                        let codec = tonic::codec::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/DiffBranch" => {
                    #[allow(non_camel_case_types)]
                    struct DiffBranchSvc<T: EventModelService>(pub Arc<T>);
                    impl<
                        T: EventModelService,
                    > tonic::server::UnaryService<super::DiffBranchRequest>
                    for DiffBranchSvc<T> {
                        type Response = super::DiffBranchResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::DiffBranchRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as EventModelService>::diff_branch(&inner, request).await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = DiffBranchSvc(inner);
                        let codec = tonic::codec::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/MergeBranch" => {
                    #[allow(non_camel_case_types)]
                    struct MergeBranchSvc<T: EventModelService>(pub Arc<T>);
                    impl<
                        T: EventModelService,
                    > tonic::server::UnaryService<super::MergeBranchRequest>
                    for MergeBranchSvc<T> {
                        type Response = super::MergeBranchResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::MergeBranchRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as EventModelService>::merge_branch(&inner, request)
                                    .await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = MergeBranchSvc(inner);
                        let codec = tonic::codec::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/UpdateBranch" => {
                    #[allow(non_camel_case_types)]
                    struct UpdateBranchSvc<T: EventModelService>(pub Arc<T>);
                    impl<
                        T: EventModelService,
                    > tonic::server::UnaryService<super::UpdateBranchRequest>
                    for UpdateBranchSvc<T> {
                        type Response = super::UpdateBranchResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::UpdateBranchRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as EventModelService>::update_branch(&inner, request)
                                    .await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = UpdateBranchSvc(inner);
                        let codec = tonic::codec::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/ResolveBranchEntry" => {
                    #[allow(non_camel_case_types)]
                    struct ResolveBranchEntrySvc<T: EventModelService>(pub Arc<T>);
                    impl<
                        T: EventModelService,
                    > tonic::server::UnaryService<super::ResolveBranchEntryRequest>
                    for ResolveBranchEntrySvc<T> {
                        type Response = super::ResolveBranchEntryResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::ResolveBranchEntryRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as EventModelService>::resolve_branch_entry(
                                        &inner,
                                        request,
                                    )
                                    .await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = ResolveBranchEntrySvc(inner);
                        let codec = tonic::codec::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/CompileTypeLibrary" => {
                    #[allow(non_camel_case_types)]
                    struct CompileTypeLibrarySvc<T: EventModelService>(pub Arc<T>);
                    impl<
                        T: EventModelService,
                    > tonic::server::UnaryService<super::CompileTypeLibraryRequest>
                    for CompileTypeLibrarySvc<T> {
                        type Response = super::CompileTypeLibraryResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::CompileTypeLibraryRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as EventModelService>::compile_type_library(
                                        &inner,
                                        request,
                                    )
                                    .await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = CompileTypeLibrarySvc(inner);
                        let codec = tonic::codec::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/ResolveType" => {
                    #[allow(non_camel_case_types)]
                    struct ResolveTypeSvc<T: EventModelService>(pub Arc<T>);
                    impl<
                        T: EventModelService,
                    > tonic::server::UnaryService<super::ResolveTypeRequest>
                    for ResolveTypeSvc<T> {
                        type Response = super::ResolveTypeResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::ResolveTypeRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as EventModelService>::resolve_type(&inner, request)
                                    .await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = ResolveTypeSvc(inner);
                        let codec = tonic::codec::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/GetTypeLibraryDescriptorSet" => {
                    #[allow(non_camel_case_types)]
                    struct GetTypeLibraryDescriptorSetSvc<T: EventModelService>(
                        pub Arc<T>,
                    );
                    impl<
                        T: EventModelService,
                    > tonic::server::UnaryService<
                        super::GetTypeLibraryDescriptorSetRequest,
                    > for GetTypeLibraryDescriptorSetSvc<T> {
                        type Response = super::GetTypeLibraryDescriptorSetResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<
                                super::GetTypeLibraryDescriptorSetRequest,
                            >,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as EventModelService>::get_type_library_descriptor_set(
                                        &inner,
                                        request,
                                    )
                                    .await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = GetTypeLibraryDescriptorSetSvc(inner);
                        let codec = tonic::codec::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                _ => {
                    Box::pin(async move {
                        Ok(
                            http::Response::builder()
                                .status(200)
                                .header("grpc-status", tonic::Code::Unimplemented as i32)
                                .header(
                                    http::header::CONTENT_TYPE,
                                    tonic::metadata::GRPC_CONTENT_TYPE,
                                )
                                .body(empty_body())
                                .unwrap(),
                        )
                    })
                }
            }
        }
    }
    impl<T: EventModelService> Clone for EventModelServiceServer<T> {
        fn clone(&self) -> Self {
            let inner = self.inner.clone();
            Self {
                inner,
                accept_compression_encodings: self.accept_compression_encodings,
                send_compression_encodings: self.send_compression_encodings,
                max_decoding_message_size: self.max_decoding_message_size,
                max_encoding_message_size: self.max_encoding_message_size,
            }
        }
    }
    impl<T: EventModelService> tonic::server::NamedService
    for EventModelServiceServer<T> {
        const NAME: &'static str = "trogonatlas.api.eventmodel.v1alpha1.EventModelService";
    }
}
