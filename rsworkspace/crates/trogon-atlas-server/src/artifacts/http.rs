use std::time::Duration;

use axum::{
    body::{to_bytes, Body},
    extract::{DefaultBodyLimit, Path, Request, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, put},
    Router,
};
use trogon_atlas_client::client::{client_for_transport, ConnectOptions, Transport};
use trogon_atlas_proto as pb;

use crate::mcp::DataPlane;

#[derive(Clone)]
struct HttpState {
    transport: Transport,
    max_bytes: usize,
}

pub fn router(data_plane: DataPlane, max_bytes: usize) -> Router {
    Router::new()
        .route("/api/artifact-uploads/{namespace}/{id}", put(upload))
        .route("/api/artifacts/{namespace}/{id}/content", get(content))
        .layer(DefaultBodyLimit::max(max_bytes))
        .with_state(HttpState {
            transport: Transport::new(data_plane),
            max_bytes,
        })
}

fn client(
    state: &HttpState,
    headers: &HeaderMap,
) -> Result<trogon_atlas_client::client::Client, Response> {
    let token = match headers.get("authorization") {
        None => None,
        Some(value) => Some(
            value
                .to_str()
                .ok()
                .and_then(|value| value.strip_prefix("Bearer "))
                .filter(|value| !value.is_empty())
                .ok_or_else(|| StatusCode::UNAUTHORIZED.into_response())?,
        ),
    };
    client_for_transport(
        state.transport.clone(),
        token,
        &ConnectOptions {
            max_message_bytes: Some(state.max_bytes + 65536),
            ..Default::default()
        },
    )
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

fn error(status: &tonic::Status) -> Response {
    let code = match status.code() {
        tonic::Code::Unauthenticated => StatusCode::UNAUTHORIZED,
        tonic::Code::PermissionDenied => StatusCode::FORBIDDEN,
        tonic::Code::NotFound => StatusCode::NOT_FOUND,
        tonic::Code::InvalidArgument => StatusCode::BAD_REQUEST,
        tonic::Code::FailedPrecondition | tonic::Code::Aborted => StatusCode::CONFLICT,
        tonic::Code::ResourceExhausted => StatusCode::PAYLOAD_TOO_LARGE,
        tonic::Code::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (code, "artifact request failed").into_response()
}

async fn upload(
    State(state): State<HttpState>,
    Path((namespace, id)): Path<(String, String)>,
    headers: HeaderMap,
    request: Request,
) -> Response {
    let mut client = match client(&state, &headers) {
        Ok(client) => client,
        Err(error) => return error,
    };
    match tokio::time::timeout(Duration::from_secs(60), async {
        let identity = client
            .who_am_i(pb::WhoAmIRequest {})
            .await
            .map_err(|status| error(&status))?
            .into_inner();
        if identity.role != pb::Role::Writer as i32 && identity.role != pb::Role::Admin as i32 {
            return Err(StatusCode::FORBIDDEN.into_response());
        }
        let body = to_bytes(request.into_body(), state.max_bytes)
            .await
            .map_err(|_| StatusCode::PAYLOAD_TOO_LARGE.into_response())?;
        client
            .upload_artifact_content(pb::UploadArtifactContentRequest {
                namespace,
                upload_id: id,
                content: body.to_vec(),
            })
            .await
            .map_err(|status| error(&status))?;
        Ok::<_, Response>(())
    })
    .await
    {
        Ok(Ok(())) => StatusCode::NO_CONTENT.into_response(),
        Ok(Err(error)) => error,
        Err(_) => StatusCode::GATEWAY_TIMEOUT.into_response(),
    }
}

async fn content(
    State(state): State<HttpState>,
    Path((namespace, id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    let mut client = match client(&state, &headers) {
        Ok(client) => client,
        Err(error) => return error,
    };
    let result = tokio::time::timeout(
        Duration::from_secs(60),
        client.get_artifact_content(pb::GetArtifactContentRequest {
            namespace,
            artifact_id: id,
        }),
    )
    .await;
    let response = match result {
        Ok(Ok(response)) => response.into_inner(),
        Ok(Err(status)) => return error(&status),
        Err(_) => return StatusCode::GATEWAY_TIMEOUT.into_response(),
    };
    let Some(artifact) = response.artifact else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let inline = matches!(
        artifact.media_type.as_str(),
        "image/png" | "image/jpeg" | "image/webp" | "image/gif"
    );
    let mut output = Body::from(response.content).into_response();
    if let Ok(media_type) = artifact.media_type.parse() {
        output.headers_mut().insert("content-type", media_type);
    }
    let headers = output.headers_mut();
    headers.insert(
        "content-disposition",
        if inline { "inline" } else { "attachment" }
            .parse()
            .unwrap_or_else(|_| axum::http::HeaderValue::from_static("attachment")),
    );
    headers.insert(
        "x-content-type-options",
        axum::http::HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        "content-security-policy",
        axum::http::HeaderValue::from_static("default-src 'none'; sandbox"),
    );
    headers.insert(
        "cache-control",
        axum::http::HeaderValue::from_static("private, no-store"),
    );
    output
}
