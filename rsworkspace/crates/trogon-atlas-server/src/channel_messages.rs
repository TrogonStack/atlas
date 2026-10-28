//! Inverse discovery preserves each native declaration and its authority.

use sha2::{Digest, Sha256};
use tonic::Status;
use trogon_atlas_core::{message_claims, resource_associations};

use super::{entity_id, entity_kind, pb, EventModelServiceImpl, Visibility};
use crate::validation::architecture_resources::ArchitectureContext;

impl EventModelServiceImpl {
    pub(super) async fn query_channel_messages(
        &self,
        principal: Option<&crate::auth::Principal>,
        branch: Option<&str>,
        vis: &Visibility,
        request: pb::ListChannelMessagesRequest,
    ) -> Result<pb::ListChannelMessagesResponse, Status> {
        if branch.is_some_and(|name| !Self::branch_visible(principal, name)) {
            return Err(unavailable());
        }
        let channel_id = request
            .channel
            .as_ref()
            .and_then(|r| r.id.clone())
            .ok_or_else(|| Status::invalid_argument("channel is required"))?;
        super::decision_document_queries::exact(&pb::EntityRef {
            kind: pb::EntityKind::Channel as i32,
            id: Some(channel_id.clone()),
        })?;
        let _guard = self.lock_mutations().await;
        let mut query = self.catalog_query(branch, vis, true).await?;
        if !query.lens.admits(&channel_id.namespace) {
            return Err(unavailable());
        }
        let channel = self
            .catalog_read(
                &mut query,
                &pb::EntityRef {
                    kind: pb::EntityKind::Channel as i32,
                    id: Some(channel_id.clone()),
                },
            )
            .await?
            .ok_or_else(unavailable)?;
        if !matches!(channel.entity.kind, Some(pb::entity::Kind::Channel(_))) {
            return Err(unavailable());
        }
        let rows = query.rows.clone();
        // Hydrate exact selected specifications through the same authorized cache.
        for row in &rows {
            query.budget.owner(&row.entity)?;
            query.budget.channel_specification(&row.entity)?;
            for authority in message_claims::routes_for_channel(
                &row.entity,
                &pb::ChannelRef {
                    id: Some(channel_id.clone()),
                },
            )
            .map_err(|_| Status::data_loss("stored claim route inventory is invalid"))?
            {
                if let Some(reference) = authority
                    .claim
                    .message
                    .as_ref()
                    .and_then(|m| m.specification.as_ref())
                {
                    if reference
                        .id
                        .as_ref()
                        .is_some_and(|id| query.lens.admits(&id.namespace))
                    {
                        self.catalog_read(
                            &mut query,
                            &pb::EntityRef {
                                kind: pb::EntityKind::Specification as i32,
                                id: reference.id.clone(),
                            },
                        )
                        .await?;
                    }
                }
            }
        }
        let context = ArchitectureContext::new(query.cached_rows());
        let mut records = Vec::new();
        for row in &rows {
            let owner = &row.entity;
            let owner_ref = pb::EntityRef {
                kind: entity_kind(owner)? as i32,
                id: Some(entity_id(owner)?.clone()),
            };
            let uid = owner
                .system
                .as_ref()
                .map(|s| s.uid.clone())
                .unwrap_or_default();
            if let Some(pb::entity::Kind::Specification(spec)) = &owner.kind {
                for declaration in &spec.message_channels {
                    if declaration.channel.as_ref().and_then(|r| r.id.as_ref()) != Some(&channel_id)
                    {
                        continue;
                    }
                    let message = resource_associations::declared_message_ref(owner, declaration);
                    let resolved = message.as_ref().is_some_and(|m| {
                        trogon_atlas_core::specification::resolve_member(owner, m).is_ok()
                    });
                    records.push(pb::ChannelMessageRecord {
                        owner: Some(owner_ref.clone()),
                        owner_uid: uid.clone(),
                        connection_id: declaration.connection_id.clone(),
                        message,
                        channel: declaration.channel.clone(),
                        parameter_values: declaration.parameter_values.clone(),
                        delivery_mode: declaration.delivery_mode,
                        doc: declaration.doc.clone(),
                        authority: pb::ChannelMessageAuthority::SpecificationDeclaration as i32,
                        state: state(resolved),
                        ..Default::default()
                    });
                }
                for operation in &spec.operations {
                    let Some(pb::operation_binding::Kind::Channel(binding)) =
                        operation.binding.as_ref().and_then(|b| b.kind.as_ref())
                    else {
                        continue;
                    };
                    if binding.channel.as_ref().and_then(|r| r.id.as_ref()) != Some(&channel_id) {
                        continue;
                    }
                    for occurrence in &operation.messages {
                        let message = pb::ContractMemberRef {
                            specification: Some(pb::SpecificationRef {
                                id: spec.id.clone(),
                            }),
                            owner_uid: uid.clone(),
                            kind: pb::ContractMemberKind::Message as i32,
                            member_id: occurrence.message_id.clone(),
                            expected_digest: None,
                        };
                        let resolved =
                            trogon_atlas_core::specification::resolve_member(owner, &message)
                                .is_ok();
                        records.push(pb::ChannelMessageRecord {
                            owner: Some(owner_ref.clone()),
                            owner_uid: uid.clone(),
                            connection_id: binding.connection_id.clone(),
                            message: Some(message),
                            channel: binding.channel.clone(),
                            operation: Some(pb::ContractMemberRef {
                                specification: Some(pb::SpecificationRef {
                                    id: spec.id.clone(),
                                }),
                                owner_uid: uid.clone(),
                                kind: pb::ContractMemberKind::Operation as i32,
                                member_id: operation.member_id.clone(),
                                expected_digest: None,
                            }),
                            operation_message: Some(occurrence.clone()),
                            authority: pb::ChannelMessageAuthority::OperationBinding as i32,
                            state: state(resolved),
                            ..Default::default()
                        });
                    }
                }
            }
            for authority in message_claims::routes_for_channel(
                owner,
                &pb::ChannelRef {
                    id: Some(channel_id.clone()),
                },
            )
            .map_err(|_| Status::data_loss("stored claim route inventory is invalid"))?
            {
                let resolved = crate::message_claims_resources::authorized_route_authority(
                    owner,
                    authority.route,
                    &context,
                )
                .is_ok();
                records.push(pb::ChannelMessageRecord {
                    owner: Some(owner_ref.clone()),
                    owner_uid: uid.clone(),
                    connection_id: authority.route.connection_id.clone(),
                    message: authority.claim.message.clone(),
                    channel: authority.route.channel.clone(),
                    parameter_values: authority.route.parameter_values.clone(),
                    delivery_mode: authority.route.delivery_mode,
                    doc: authority.route.doc.as_ref().map(|v| v.value.clone()),
                    claim: Some(authority.claim.clone()),
                    route: Some(authority.route.clone()),
                    authority: pb::ChannelMessageAuthority::ClaimRoute as i32,
                    state: state(resolved),
                    ..Default::default()
                });
            }
        }
        drop(context);
        records.sort_by_key(record_key);
        let caller = caller_key(principal);
        let scope = query.checked_scope(Some(&caller))?;
        let digest = scope.revision;
        let start = parse_token(&request.page_token, &channel_id, branch, &digest, &caller)?;
        if start > records.len() || (!request.page_token.is_empty() && start == records.len()) {
            return Err(rejected_token());
        }
        let page_size = if request.page_size == 0 {
            100
        } else {
            (request.page_size as usize).min(500)
        };
        let end = start.saturating_add(page_size).min(records.len());
        let complete = end == records.len();
        let records: Vec<_> = records.drain(start..end).collect();
        for _ in &records {
            query.budget.output()?;
        }
        Ok(pb::ListChannelMessagesResponse {
            records,
            next_page_token: if complete {
                String::new()
            } else {
                encode_token(&channel_id, branch, &digest, &caller, end)?
            },
            incomplete: false,
            complete,
            branch: branch.map(str::to_owned),
            scope_digest: digest,
            authorized_namespace_count: scope
                .authorized_namespaces
                .len()
                .try_into()
                .unwrap_or(u32::MAX),
        })
    }
}

fn state(resolved: bool) -> i32 {
    if resolved {
        pb::CatalogResolutionState::Resolved as i32
    } else {
        pb::CatalogResolutionState::Unavailable as i32
    }
}

fn record_key(
    record: &pb::ChannelMessageRecord,
) -> (i32, String, String, u64, i32, String, String) {
    let owner = record.owner.as_ref();
    let id = owner.and_then(|r| r.id.as_ref());
    (
        owner.map_or(0, |r| r.kind),
        id.map_or_else(String::new, |id| id.namespace.clone()),
        id.map_or_else(String::new, |id| id.slug.clone()),
        id.map_or(0, |id| id.version),
        record.authority,
        record
            .connection_id
            .as_ref()
            .map_or_else(String::new, |id| id.value.clone()),
        record
            .operation_message
            .as_ref()
            .and_then(|m| m.connection_id.as_ref())
            .map_or_else(String::new, |id| id.value.clone()),
    )
}

fn unavailable() -> Status {
    Status::not_found("resource unavailable")
}
fn rejected_token() -> Status {
    Status::invalid_argument("page token does not match this query")
}
fn caller_key(principal: Option<&crate::auth::Principal>) -> String {
    Sha256::digest(format!("{principal:?}").as_bytes())
        .iter()
        .fold(String::new(), |mut value, byte| {
            const HEX: &[u8; 16] = b"0123456789abcdef";
            value.push(char::from(HEX[usize::from(byte >> 4)]));
            value.push(char::from(HEX[usize::from(byte & 15)]));
            value
        })
}
fn encode_token(
    channel: &pb::Id,
    branch: Option<&str>,
    digest: &str,
    caller: &str,
    offset: usize,
) -> Result<String, Status> {
    serde_json::to_string(&(
        2,
        &channel.namespace,
        &channel.slug,
        channel.version,
        branch,
        digest,
        caller,
        offset,
    ))
    .map_err(|_| Status::internal("cannot encode channel page token"))
}
fn parse_token(
    token: &str,
    channel: &pb::Id,
    branch: Option<&str>,
    digest: &str,
    caller: &str,
) -> Result<usize, Status> {
    if token.is_empty() {
        return Ok(0);
    }
    if token.len() > 4096 {
        return Err(rejected_token());
    }
    let (version, ns, slug, rev, selected_branch, checked, identity, offset): (
        u8,
        String,
        String,
        u64,
        Option<String>,
        String,
        String,
        usize,
    ) = serde_json::from_str(token).map_err(|_| rejected_token())?;
    if version != 2
        || ns != channel.namespace
        || slug != channel.slug
        || rev != channel.version
        || selected_branch.as_deref() != branch
        || checked != digest
        || identity != caller
        || offset == 0
    {
        return Err(rejected_token());
    }
    Ok(offset)
}
