use prost::Message;
use sha2::{Digest, Sha256};

use super::{pb, store_err, EventModelServiceImpl, Status, Visibility};

impl EventModelServiceImpl {
    pub(super) async fn term_domains(
        &self,
        req: pb::ListTermDomainDeclarationsRequest,
        branch: Option<&str>,
        vis: &Visibility,
    ) -> Result<pb::ListTermDomainDeclarationsResponse, Status> {
        let domain = req
            .domain
            .ok_or_else(|| Status::invalid_argument("domain is required"))?;
        let id = super::require_id(domain.id.as_ref(), "domain.id")?;
        if id.version == 0 || req.page_size > 500 {
            return Err(Status::invalid_argument(
                "exact domain and page size at most 500 required",
            ));
        }
        if let Some(branch) = branch {
            self.require_branch(branch).await?;
        }
        self.require_visible(vis, &id.namespace).await?;
        let mut target_revision = String::new();
        let state = match self.store.get(pb::EntityKind::Domain, id, branch).await {
            Ok(row) => {
                target_revision = row.etag;
                pb::CatalogResolutionState::Resolved
            }
            Err(trogon_atlas_store::error::StoreError::NotFound) => {
                pb::CatalogResolutionState::Unavailable
            }
            Err(error) => return Err(store_err(error)),
        };
        let rows = self
            .architecture_reference_snapshot(&[], branch, vis)
            .await?;
        let mut keyed = rows
            .into_iter()
            .map(|row| {
                let id = super::entity_id(&row.entity)?;
                let key = (
                    super::entity_kind(&row.entity)? as i32,
                    id.namespace.clone(),
                    id.slug.clone(),
                    id.version,
                );
                Ok((key, row))
            })
            .collect::<Result<Vec<_>, Status>>()?;
        keyed.sort_by(|a, b| a.0.cmp(&b.0));
        let mut budget = crate::decision_document_resources::CatalogBudget::default();
        let mut hash = Sha256::new();
        hash.update(format!("{vis:?}:{branch:?}"));
        hash.update(domain.encode_to_vec());
        hash.update(target_revision.as_bytes());
        let mut declarations = Vec::new();
        for (_, row) in keyed {
            budget.owner(&row.entity)?;
            hash.update(row.entity.encode_to_vec());
            hash.update(row.etag.as_bytes());
            if let Some(pb::entity::Kind::Term(term)) = &row.entity.kind {
                budget.examine(1)?;
                if term.owning_domain.as_ref() == Some(&domain) {
                    declarations.push(pb::TermDomainDeclaration {
                        owner: Some(pb::CatalogCheckedOwner {
                            owner: Some(crate::conv::entity_to_ref(&row.entity)?),
                            uid: row
                                .entity
                                .system
                                .as_ref()
                                .map_or_else(String::new, |s| s.uid.clone()),
                            etag: row.etag,
                        }),
                        domain: Some(domain.clone()),
                        state: state as i32,
                    });
                }
            }
        }
        let binding = hash
            .finalize()
            .iter()
            .fold(String::new(), |mut text, byte| {
                const HEX: &[u8; 16] = b"0123456789abcdef";
                text.push(char::from(HEX[usize::from(byte >> 4)]));
                text.push(char::from(HEX[usize::from(byte & 15)]));
                text
            });
        let offset = if req.page_token.is_empty() {
            0
        } else {
            let (token_binding, offset) = req
                .page_token
                .split_once(':')
                .ok_or_else(|| Status::invalid_argument("invalid page token"))?;
            if token_binding != binding {
                return Err(Status::invalid_argument("stale or mismatched page token"));
            }
            offset
                .parse::<usize>()
                .map_err(|_| Status::invalid_argument("invalid page token"))?
        };
        if offset > declarations.len() {
            return Err(Status::invalid_argument("invalid page token"));
        }
        let size = if req.page_size == 0 {
            100
        } else {
            req.page_size as usize
        };
        let end = offset.saturating_add(size).min(declarations.len());
        for _ in offset..end {
            budget.output()?;
        }
        Ok(pb::ListTermDomainDeclarationsResponse {
            next_page_token: if end < declarations.len() {
                format!("{binding}:{end}")
            } else {
                String::new()
            },
            declarations: declarations[offset..end].to_vec(),
            incomplete: false,
        })
    }
}
