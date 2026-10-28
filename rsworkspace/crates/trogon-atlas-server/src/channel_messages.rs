//! Inverse discovery of independent specification message-to-channel
//! declarations. Claim routes and operation bindings are not scanned.

use sha2::{Digest, Sha256};
use tonic::Status;

use super::{pb, store_err, EventModelServiceImpl, Lens, ListFilter, Visibility};

const MAX_ROWS: usize = 5_000;
const MAX_OCCURRENCES: usize = 10_000;
const MAX_EMITTED: usize = 4_096;
const DEFAULT_PAGE_SIZE: usize = 100;
const MAX_PAGE_SIZE: usize = 500;
const SPEC_SCAN_LIMIT: usize = 5_000;

struct Cursor {
    namespace: String,
    slug: String,
    version: u64,
    connection_id: String,
}

impl EventModelServiceImpl {
    pub(super) async fn query_channel_messages(
        &self,
        principal: Option<&crate::auth::Principal>,
        branch: Option<&str>,
        vis: &Visibility,
        request: pb::ListChannelMessagesRequest,
    ) -> Result<pb::ListChannelMessagesResponse, Status> {
        if let Some(name) = branch {
            if !Self::branch_visible(principal, name) {
                return Err(unavailable());
            }
        }
        let channel_id = request
            .channel
            .as_ref()
            .and_then(|channel| channel.id.clone())
            .ok_or_else(|| Status::invalid_argument("channel is required"))?;
        if channel_id.version == 0
            || !trogon_atlas_core::is_safe_id_component(&channel_id.namespace)
            || !trogon_atlas_core::is_safe_id_component(&channel_id.slug)
        {
            return Err(Status::invalid_argument("channel reference is invalid"));
        }
        let lens = self.lens(vis).await?;
        if !lens.admits(&channel_id.namespace) {
            return Err(unavailable());
        }
        let stored = match self
            .store
            .get(pb::EntityKind::Channel, &channel_id, branch)
            .await
        {
            Ok(stored) => stored,
            Err(trogon_atlas_store::error::StoreError::NotFound) => return Err(unavailable()),
            Err(error) => return Err(store_err(error)),
        };
        if !matches!(stored.entity.kind, Some(pb::entity::Kind::Channel(_))) {
            return Err(unavailable());
        }
        let (digest, namespace_count, namespaces) = scope(&lens, branch);
        let caller = caller_key(principal);
        let cursor = parse_token(&request.page_token, &channel_id, branch, &digest, &caller)?;
        let page_size = match request.page_size {
            0 => DEFAULT_PAGE_SIZE,
            size if size as usize > MAX_PAGE_SIZE => MAX_PAGE_SIZE,
            size => size as usize,
        };
        let specs = self
            .store
            .list(
                ListFilter {
                    kinds: &[pb::EntityKind::Specification],
                    namespaces: &namespaces,
                    latest_versions_only: false,
                },
                Some(SPEC_SCAN_LIMIT),
                branch,
            )
            .await
            .map_err(store_err)?;
        let mut rows = 1usize;
        let mut occurrences = 0usize;
        let mut selected = Vec::new();
        for stored in &specs {
            let Some(pb::entity::Kind::Specification(specification)) = &stored.entity.kind else {
                continue;
            };
            let Some(spec_id) = specification.id.as_ref() else {
                continue;
            };
            if !lens.admits(&spec_id.namespace) {
                continue;
            }
            rows = rows.saturating_add(1);
            if rows > MAX_ROWS {
                return Err(Status::resource_exhausted(
                    "channel message budget exhausted",
                ));
            }
            occurrences = occurrences.saturating_add(specification.message_channels.len());
            if occurrences > MAX_OCCURRENCES {
                return Err(Status::resource_exhausted(
                    "channel message budget exhausted",
                ));
            }
            let mut matched: Vec<_> = specification
                .message_channels
                .iter()
                .filter(|declaration| {
                    declaration
                        .channel
                        .as_ref()
                        .and_then(|channel| channel.id.as_ref())
                        == Some(&channel_id)
                })
                .collect();
            matched.sort_by(|left, right| connection_value(left).cmp(&connection_value(right)));
            for declaration in matched {
                selected.push(Selected {
                    entity: &stored.entity,
                    spec_id,
                    declaration,
                    connection_id: connection_value(declaration),
                });
            }
        }
        if specs.len() == SPEC_SCAN_LIMIT {
            return Err(Status::resource_exhausted(
                "channel message budget exhausted",
            ));
        }
        let mut seen_matches = 0usize;
        let mut records = Vec::new();
        let mut last: Option<Cursor> = None;
        let mut stopped_for_page = false;
        let mut hit_emit_cap = false;
        for item in &selected {
            seen_matches = seen_matches.saturating_add(1);
            if seen_matches > MAX_EMITTED {
                hit_emit_cap = true;
                break;
            }
            if !after(item.spec_id, &item.connection_id, cursor.as_ref()) {
                continue;
            }
            if records.len() == page_size {
                stopped_for_page = true;
                break;
            }
            records.push(pb::ChannelMessageRecord {
                owner: Some(pb::EntityRef {
                    kind: pb::EntityKind::Specification as i32,
                    id: Some(item.spec_id.clone()),
                }),
                owner_uid: item
                    .entity
                    .system
                    .as_ref()
                    .map(|system| system.uid.clone())
                    .unwrap_or_default(),
                connection_id: item.declaration.connection_id.clone(),
                message: trogon_atlas_core::resource_associations::declared_message_ref(
                    item.entity,
                    item.declaration,
                ),
                channel: item.declaration.channel.clone(),
                parameter_values: item.declaration.parameter_values.clone(),
                delivery_mode: item.declaration.delivery_mode,
                doc: item.declaration.doc.clone(),
                authority: pb::ChannelMessageAuthority::SpecificationDeclaration as i32,
            });
            last = Some(Cursor {
                namespace: item.spec_id.namespace.clone(),
                slug: item.spec_id.slug.clone(),
                version: item.spec_id.version,
                connection_id: item.connection_id.clone(),
            });
        }
        let (next_page_token, incomplete, complete) = if hit_emit_cap || seen_matches >= MAX_EMITTED
        {
            (String::new(), true, false)
        } else if stopped_for_page {
            (
                encode_token(&channel_id, branch, &digest, &caller, last.as_ref()),
                false,
                false,
            )
        } else {
            (String::new(), false, true)
        };
        Ok(pb::ListChannelMessagesResponse {
            records,
            next_page_token,
            incomplete,
            complete,
            branch: branch.map(str::to_string),
            scope_digest: digest,
            authorized_namespace_count: namespace_count,
        })
    }
}

fn unavailable() -> Status {
    Status::not_found("resource unavailable")
}

fn scope(lens: &Lens, branch: Option<&str>) -> (String, u32, Vec<String>) {
    let mut namespaces = match lens {
        Lens::Everything => Vec::new(),
        Lens::Only(ids) => ids.iter().map(|id| id.as_str().to_string()).collect(),
    };
    namespaces.sort();
    let mut hasher = Sha256::new();
    hasher.update(branch.unwrap_or(""));
    hasher.update([0]);
    if matches!(lens, Lens::Everything) {
        hasher.update(b"*");
    }
    for namespace in &namespaces {
        hasher.update(namespace.as_bytes());
        hasher.update([0]);
    }
    let digest = hex(&hasher.finalize());
    let count = u32::try_from(namespaces.len()).unwrap_or(u32::MAX);
    (digest, count, namespaces)
}

struct Selected<'a> {
    entity: &'a pb::Entity,
    spec_id: &'a pb::Id,
    declaration: &'a pb::MessageChannelDeclaration,
    connection_id: String,
}

fn caller_key(principal: Option<&crate::auth::Principal>) -> String {
    let mut hasher = Sha256::new();
    hasher.update(
        principal
            .map(|principal| principal.name.as_ref())
            .unwrap_or(""),
    );
    hex(&hasher.finalize())
}

fn connection_value(declaration: &pb::MessageChannelDeclaration) -> String {
    declaration
        .connection_id
        .as_ref()
        .map(|id| id.value.clone())
        .unwrap_or_default()
}

fn after(spec_id: &pb::Id, connection_id: &str, cursor: Option<&Cursor>) -> bool {
    let Some(cursor) = cursor else {
        return true;
    };
    (
        spec_id.namespace.as_str(),
        spec_id.slug.as_str(),
        spec_id.version,
        connection_id,
    ) > (
        cursor.namespace.as_str(),
        cursor.slug.as_str(),
        cursor.version,
        cursor.connection_id.as_str(),
    )
}

fn encode_token(
    channel: &pb::Id,
    branch: Option<&str>,
    digest: &str,
    caller: &str,
    cursor: Option<&Cursor>,
) -> String {
    let Some(cursor) = cursor else {
        return String::new();
    };
    format!(
        "v1\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}",
        channel.namespace,
        channel.slug,
        channel.version,
        branch.unwrap_or(""),
        digest,
        caller,
        cursor.namespace,
        cursor.slug,
        cursor.version,
        cursor.connection_id
    )
}

fn parse_token(
    token: &str,
    channel: &pb::Id,
    branch: Option<&str>,
    digest: &str,
    caller: &str,
) -> Result<Option<Cursor>, Status> {
    if token.is_empty() {
        return Ok(None);
    }
    let parts: Vec<&str> = token.split('\n').collect();
    if parts.len() != 11 || parts[0] != "v1" {
        return Err(rejected_token());
    }
    let version: u64 = parts[3].parse().map_err(|_| rejected_token())?;
    let cursor_version: u64 = parts[9].parse().map_err(|_| rejected_token())?;
    if parts[1] != channel.namespace
        || parts[2] != channel.slug
        || version != channel.version
        || parts[4] != branch.unwrap_or("")
        || parts[5] != digest
        || parts[6] != caller
    {
        return Err(rejected_token());
    }
    Ok(Some(Cursor {
        namespace: parts[7].to_string(),
        slug: parts[8].to_string(),
        version: cursor_version,
        connection_id: parts[10].to_string(),
    }))
}

fn rejected_token() -> Status {
    Status::invalid_argument("page token does not match this query")
}

fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(char::from(HEX[usize::from(byte >> 4)]));
        out.push(char::from(HEX[usize::from(byte & 15)]));
    }
    out
}
