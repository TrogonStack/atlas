use std::{
    future::Future,
    ops::Deref,
    pin::Pin,
    sync::{Arc, LazyLock},
    task::{Context, Poll},
};

use bytes::Bytes;
use http_body::{Body, Frame, SizeHint};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tonic::Status;
use tower::Service;
use trogon_atlas_proto as pb;

// Account for provider growth/chunks, the response Vec, codec growth and a
// subsequent wire-format conversion. This is a payload budget, not process RSS.
const MEMORY_BYTES: usize = 256 * 1024 * 1024;
const PAYLOAD_COPIES: u64 = 6;
const TRANSFER_OVERHEAD: u64 = 128 * 1024;
static MEMORY: LazyLock<Arc<Semaphore>> = LazyLock::new(|| Arc::new(Semaphore::new(MEMORY_BYTES)));

#[derive(Clone)]
pub(super) struct DownloadLease {
    _permit: Arc<OwnedSemaphorePermit>,
}

impl DownloadLease {
    pub(super) fn reserve(size: u64) -> Result<Self, Status> {
        if size == 0 || size > super::MAX_ARTIFACT_BYTES {
            return Err(Status::failed_precondition(
                "invalid retained artifact size",
            ));
        }
        let charge = size
            .checked_mul(PAYLOAD_COPIES)
            .and_then(|bytes| bytes.checked_add(TRANSFER_OVERHEAD))
            .and_then(|bytes| u32::try_from(bytes).ok())
            .ok_or_else(|| Status::failed_precondition("artifact size exceeds budget capacity"))?;
        let permit = MEMORY.clone().try_acquire_many_owned(charge).map_err(|_| {
            Status::resource_exhausted(
                "artifact download memory capacity is exhausted; retry after active transfers finish",
            )
        })?;
        Ok(Self {
            _permit: Arc::new(permit),
        })
    }
}

pub struct BudgetedContent {
    message: pb::GetArtifactContentResponse,
    lease: DownloadLease,
}

impl BudgetedContent {
    pub(super) fn new(message: pb::GetArtifactContentResponse, lease: DownloadLease) -> Self {
        Self { message, lease }
    }

    #[must_use]
    pub fn into_response(self) -> tonic::Response<pb::GetArtifactContentResponse> {
        let Self { message, lease } = self;
        let mut response = tonic::Response::new(message);
        response.extensions_mut().insert(lease);
        response
    }
}

impl Deref for BudgetedContent {
    type Target = pb::GetArtifactContentResponse;

    fn deref(&self) -> &Self::Target {
        &self.message
    }
}

// Payload fields precede leases so buffers drop before their budget is released.
struct FrameOwner {
    data: Bytes,
    _lease: DownloadLease,
}

impl AsRef<[u8]> for FrameOwner {
    fn as_ref(&self) -> &[u8] {
        self.data.as_ref()
    }
}

struct BudgetedBody<B> {
    inner: Option<Pin<Box<B>>>,
    lease: Option<DownloadLease>,
}

impl<B: Body<Data = Bytes>> Body for BudgetedBody<B> {
    type Data = Bytes;
    type Error = B::Error;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Self::Error>>> {
        let this = self.get_mut();
        let Some(inner) = this.inner.as_mut() else {
            return Poll::Ready(None);
        };
        match inner.as_mut().poll_frame(cx) {
            Poll::Ready(Some(Ok(frame))) => {
                let Some(lease) = this.lease.as_ref() else {
                    return Poll::Ready(Some(Ok(frame)));
                };
                let frame = frame.map_data(|data| {
                    Bytes::from_owner(FrameOwner {
                        data,
                        _lease: lease.clone(),
                    })
                });
                Poll::Ready(Some(Ok(frame)))
            }
            Poll::Ready(outcome) => {
                // EOF can still leave codec capacity allocated inside the body.
                this.inner.take();
                this.lease.take();
                Poll::Ready(outcome)
            }
            Poll::Pending => Poll::Pending,
        }
    }

    fn is_end_stream(&self) -> bool {
        self.inner.as_ref().is_none_or(Body::is_end_stream)
    }

    fn size_hint(&self) -> SizeHint {
        self.inner
            .as_ref()
            .map_or_else(|| SizeHint::with_exact(0), Body::size_hint)
    }
}

#[derive(Clone)]
pub struct ResponseBudgetService<S> {
    inner: S,
}

impl<S> ResponseBudgetService<S> {
    pub(crate) fn new(inner: S) -> Self {
        Self { inner }
    }
}

impl<S, RequestBody, ResponseBody> Service<http::Request<RequestBody>> for ResponseBudgetService<S>
where
    S: Service<http::Request<RequestBody>, Response = http::Response<ResponseBody>>,
    S::Future: Send + 'static,
    S::Error: Send + 'static,
    ResponseBody: Body<Data = Bytes> + Send + 'static,
    ResponseBody::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    type Response = http::Response<tonic::body::Body>;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, request: http::Request<RequestBody>) -> Self::Future {
        let future = self.inner.call(request);
        Box::pin(async move {
            let response = future.await?;
            let (mut parts, body) = response.into_parts();
            let body = match parts.extensions.remove::<DownloadLease>() {
                Some(lease) => tonic::body::Body::new(BudgetedBody {
                    inner: Some(Box::pin(body)),
                    lease: Some(lease),
                }),
                None => tonic::body::Body::new(body),
            };
            Ok(http::Response::from_parts(parts, body))
        })
    }
}

impl<S: tonic::server::NamedService> tonic::server::NamedService for ResponseBudgetService<S> {
    const NAME: &'static str = S::NAME;
}
