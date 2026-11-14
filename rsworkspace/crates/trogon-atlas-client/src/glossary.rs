use anyhow::{Context, Result};
use serde_json::Value;
use trogon_atlas_core::transcode::TranscodePool;
use trogon_atlas_proto as pb;

pub use crate::responsibility::parse_request;
use crate::{
    client::Client,
    precondition::{Etag, Revision, StalePlan},
    tenant_types::{self, TypeScope},
};

fn response<M: prost::Message>(name: &str, message: &M) -> Result<Value> {
    TranscodePool::builtin()?
        .message_json_value(
            &format!("trogonatlas.api.eventmodel.v1alpha1.{name}"),
            message,
        )
        .with_context(|| format!("encode {name}"))
}

async fn term_response(
    client: &mut Client,
    message: &pb::GetGlossaryTermResponse,
) -> Result<Value> {
    let mut scope = TypeScope::default();
    if let Some(id) = message
        .entity
        .as_ref()
        .and_then(trogon_atlas_core::entity_id)
    {
        scope.add_namespace(id.namespace.clone());
    }
    let pool = tenant_types::transcode_pool(client, &scope).await?;
    term_json(&pool, message)
}

fn term_json(pool: &TranscodePool, message: &pb::GetGlossaryTermResponse) -> Result<Value> {
    pool.message_json_value(
        "trogonatlas.api.eventmodel.v1alpha1.GetGlossaryTermResponse",
        message,
    )
    .context("encode GetGlossaryTermResponse")
}

pub async fn list_glossary_entries(
    client: &mut Client,
    request: pb::ListGlossaryEntriesRequest,
) -> Result<Value> {
    let value = client
        .list_glossary_entries(request)
        .await
        .context("ListGlossaryEntries failed")?
        .into_inner();
    response("ListGlossaryEntriesResponse", &value)
}

pub async fn list_glossary_pointers(
    client: &mut Client,
    request: pb::ListGlossaryEntriesRequest,
) -> Result<Value> {
    let value = client
        .list_glossary_pointers(request)
        .await
        .context("ListGlossaryPointers failed")?
        .into_inner();
    response("ListGlossaryPointersResponse", &value)
}

pub async fn get_glossary_term(
    client: &mut Client,
    request: pb::GetGlossaryTermRequest,
) -> Result<Value> {
    let value = client
        .get_glossary_term(request)
        .await
        .context("GetGlossaryTerm failed")?
        .into_inner();
    term_response(client, &value).await
}

pub async fn get_glossary_definition(
    client: &mut Client,
    request: pb::GetGlossaryTermRequest,
) -> Result<Value> {
    let value = client
        .get_glossary_definition(request)
        .await
        .context("GetGlossaryDefinition failed")?
        .into_inner();
    response("GetGlossaryDefinitionResponse", &value)
}

pub async fn inspect_glossary_sources(
    client: &mut Client,
    request: pb::InspectGlossarySourcesRequest,
) -> Result<Value> {
    let value = client
        .inspect_glossary_sources(request)
        .await
        .context("InspectGlossarySources failed")?
        .into_inner();
    response("InspectGlossarySourcesResponse", &value)
}

pub async fn review_glossary_term(
    client: &mut Client,
    request: pb::ReviewGlossaryTermRequest,
) -> Result<Value> {
    let expected = Etag::new(request.if_match.clone())
        .map_err(|error| crate::failure::invalid_input(error.to_string()))?;
    let mut scope = TypeScope::default();
    if let Some(id) = request.term.as_ref().and_then(|term| term.id.as_ref()) {
        scope.add_namespace(id.namespace.clone());
    }
    let pool = tenant_types::transcode_pool(client, &scope).await?;
    if let Some(id) = request.term.as_ref().and_then(|term| term.id.as_ref()) {
        let candidate = client
            .get_entity(pb::GetEntityRequest {
                kind: pb::EntityKind::Term as i32,
                id: Some(id.clone()),
            })
            .await
            .context("load glossary review candidate")?
            .into_inner();
        if candidate.etag != expected.as_str() {
            return Err(StalePlan {
                entity: pb::EntityKey::new(pb::EntityKind::Term, id),
                expected: Revision::At(expected),
                actual: Revision::from_wire(&candidate.etag).context("invalid candidate etag")?,
            }
            .into());
        }
        let Some(pb::entity::Kind::Term(term)) = candidate
            .entity
            .as_ref()
            .and_then(|entity| entity.kind.as_ref())
        else {
            anyhow::bail!("glossary review candidate is not a Term");
        };
        trogon_atlas_core::glossary::candidate_digest_with_pool(term, &pool).map_err(|error| {
            crate::failure::invalid_input(format!(
                "glossary candidate cannot be represented by prepared descriptors; reload types before reviewing: {error}"
            ))
        })?;
    }
    let value = client
        .review_glossary_term(request)
        .await
        .context("ReviewGlossaryTerm failed")?
        .into_inner();
    term_json(&pool, &value)
}

pub async fn compare_glossaries(
    client: &mut Client,
    request: pb::CompareGlossariesRequest,
) -> Result<Value> {
    let value = client
        .compare_glossaries(request)
        .await
        .context("CompareGlossaries failed")?
        .into_inner();
    response("CompareGlossariesResponse", &value)
}

pub async fn check_glossary_usage(
    client: &mut Client,
    request: pb::CheckGlossaryUsageRequest,
) -> Result<Value> {
    let value = client
        .check_glossary_usage(request)
        .await
        .context("CheckGlossaryUsage failed")?
        .into_inner();
    response("CheckGlossaryUsageResponse", &value)
}

pub async fn check_glossary_code_usage(
    client: &mut Client,
    request: pb::CheckGlossaryUsageRequest,
) -> Result<Value> {
    let value = client
        .check_glossary_code_usage(request)
        .await
        .context("CheckGlossaryCodeUsage failed")?
        .into_inner();
    response("CheckGlossaryUsageResponse", &value)
}

pub async fn inspect_glossary_refresh(
    client: &mut Client,
    request: pb::InspectGlossaryRefreshRequest,
) -> Result<Value> {
    let value = client
        .inspect_glossary_refresh(request)
        .await
        .context("InspectGlossaryRefresh failed")?
        .into_inner();
    response("InspectGlossaryRefreshResponse", &value)
}
