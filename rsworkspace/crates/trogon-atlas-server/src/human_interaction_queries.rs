use std::collections::{BTreeSet, HashMap};

use prost::Message;
use sha2::{Digest, Sha256};
use tonic::{Code, Status};
use trogon_atlas_core::connections::{self, ConnectionOccurrence};

use super::{
    entity_id, entity_kind, pb, store_err, EventModelServiceImpl, Lens, StoredEntity, Visibility,
};
use crate::decision_document_resources::CatalogBudget;

#[derive(Default)]
struct Inspection {
    rows: HashMap<pb::EntityKey, Option<StoredEntity>>,
    inventories: HashMap<pb::EntityKey, Vec<ConnectionOccurrence>>,
    budget: CatalogBudget,
}

fn reference(selection: &pb::ConnectionRef) -> Result<&pb::EntityRef, Status> {
    let owner = selection
        .owner
        .as_ref()
        .ok_or_else(|| Status::invalid_argument("connection owner is required"))?;
    let (_, id) = super::decision_document_queries::exact(owner)?;
    if !trogon_atlas_core::is_safe_id_component(&id.namespace)
        || !trogon_atlas_core::is_safe_id_component(&id.slug)
        || selection.owner_uid.is_empty()
    {
        return Err(Status::invalid_argument(
            "full connection identity is required",
        ));
    }
    selection
        .id
        .as_ref()
        .ok_or_else(|| Status::invalid_argument("connection id is required"))?
        .value
        .parse::<connections::ConnectionId>()
        .map_err(|_| Status::invalid_argument("connection id is invalid"))?;
    Ok(owner)
}

fn required(value: Option<&pb::ConnectionRef>) -> Result<&pb::ConnectionRef, Status> {
    value.ok_or_else(|| Status::invalid_argument("complete interaction tuple is required"))
}

fn selections(
    request: &pb::InspectHumanInteractionRequest,
) -> Result<[(&pb::ConnectionRef, pb::HumanInteractionRole); 4], Status> {
    use pb::HumanInteractionRole as Role;
    let hosting = (required(request.hosting.as_ref())?, Role::HostedUi);
    let result = match request
        .path
        .as_ref()
        .ok_or_else(|| Status::invalid_argument("interaction path is required"))?
    {
        pb::inspect_human_interaction_request::Path::Issuance(path) => [
            hosting,
            (required(path.persona.as_ref())?, Role::Persona),
            (required(path.ui.as_ref())?, Role::SliceUi),
            (required(path.command.as_ref())?, Role::Command),
        ],
        pb::inspect_human_interaction_request::Path::Display(path) => [
            hosting,
            (required(path.persona.as_ref())?, Role::Persona),
            (required(path.ui.as_ref())?, Role::SliceUi),
            (required(path.source_read_model.as_ref())?, Role::ReadModel),
        ],
    };
    for (selection, _) in result {
        reference(selection)?;
    }
    Ok(result)
}

impl Inspection {
    fn inventory(&mut self, row: &StoredEntity) -> Result<&[ConnectionOccurrence], Status> {
        let key = pb::EntityKey::new(entity_kind(&row.entity)?, entity_id(&row.entity)?);
        if !self.inventories.contains_key(&key) {
            connections::validate_stored_connection_ids(&row.entity)
                .map_err(|_| Status::data_loss("stored connection inventory is invalid"))?;
            let inventory = connections::connection_occurrences(&row.entity)
                .map_err(|_| Status::data_loss("stored connection inventory is invalid"))?;
            self.budget.examine(inventory.len())?;
            self.inventories.insert(key.clone(), inventory);
        }
        self.inventories
            .get(&key)
            .map(Vec::as_slice)
            .ok_or_else(|| Status::internal("checked inventory missing"))
    }

    fn resolve(
        &mut self,
        row: &StoredEntity,
        selection: &pb::ConnectionRef,
        role: pb::HumanInteractionRole,
    ) -> Result<Option<ConnectionOccurrence>, Status> {
        let identity = selection
            .id
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("connection id is required"))?
            .value
            .parse::<connections::ConnectionId>()
            .map_err(|_| Status::invalid_argument("connection id is invalid"))?;
        let inventory = self.inventory(row)?;
        if row
            .entity
            .system
            .as_ref()
            .is_none_or(|system| system.uid != selection.owner_uid)
        {
            return Ok(None);
        }
        let occurrence = inventory.iter().find(|item| item.id == Some(identity));
        Ok(occurrence
            .filter(|item| match role {
                pb::HumanInteractionRole::HostedUi => {
                    item.path.starts_with("component.ui_hostings[") && item.path.ends_with(']')
                }
                pb::HumanInteractionRole::Persona => matches!(
                    item.path.as_str(),
                    "command_slice.persona" | "ui_slice.persona"
                ),
                pb::HumanInteractionRole::SliceUi => {
                    matches!(item.path.as_str(), "command_slice.ui" | "ui_slice.ui")
                }
                pb::HumanInteractionRole::Command => item.path == "command_slice.command",
                pb::HumanInteractionRole::ReadModel => {
                    item.path.starts_with("ui_slice.source_read_models[")
                        && item.path.ends_with(']')
                }
                pb::HumanInteractionRole::Unspecified => false,
            })
            .cloned())
    }
}

impl EventModelServiceImpl {
    async fn interaction_read(
        &self,
        query: &mut Inspection,
        reference: &pb::EntityRef,
        branch: Option<&str>,
        vis: &Visibility,
    ) -> Result<Option<StoredEntity>, Status> {
        let (kind, id) = super::decision_document_queries::exact(reference)?;
        let key = pb::EntityKey::new(kind, id);
        if let Some(row) = query.rows.get(&key) {
            return Ok(row.clone());
        }
        self.require_visible(vis, &id.namespace).await?;
        if query.rows.len() >= 5_000 {
            return Err(Status::resource_exhausted(
                "interaction row budget exceeded",
            ));
        }
        let row = match self.store.get(kind, id, branch).await {
            Ok(row) => {
                if crate::conv::entity_to_ref(&row.entity)? != *reference {
                    return Err(Status::data_loss("stored entity identity is invalid"));
                }
                if row
                    .entity
                    .system
                    .as_ref()
                    .is_none_or(|system| system.uid.is_empty())
                {
                    return Err(Status::data_loss("stored entity incarnation is invalid"));
                }
                Some(row)
            }
            Err(trogon_atlas_store::error::StoreError::NotFound) => None,
            Err(error) => return Err(store_err(error)),
        };
        query.rows.insert(key, row.clone());
        Ok(row)
    }

    pub(super) async fn human_interaction(
        &self,
        request: pb::InspectHumanInteractionRequest,
        branch: Option<&str>,
        vis: &Visibility,
    ) -> Result<pb::InspectHumanInteractionResponse, Status> {
        if let Some(branch) = branch {
            self.require_branch(branch).await?;
        }
        let selected = selections(&request)?;
        let mut query = Inspection::default();
        let mut owners = Vec::new();
        for (selection, _) in selected {
            let row = self
                .interaction_read(&mut query, reference(selection)?, branch, vis)
                .await?
                .ok_or_else(|| Status::not_found("entity not found"))?;
            owners.push(row);
        }
        let slice_kind = match request.path.as_ref() {
            Some(pb::inspect_human_interaction_request::Path::Issuance(_)) => {
                pb::EntityKind::CommandSlice
            }
            _ => pb::EntityKind::UiSlice,
        };
        let same_slice = selected[1..].iter().all(|(selection, _)| {
            selection.owner == selected[1].0.owner && selection.owner_uid == selected[1].0.owner_uid
        });
        let kinds_match = reference(selected[0].0)?.kind == pb::EntityKind::Component as i32
            && reference(selected[1].0)?.kind == slice_kind as i32;
        let mut occurrences = Vec::new();
        if same_slice && kinds_match {
            for ((selection, role), owner) in selected.iter().zip(&owners) {
                if let Some(occurrence) = query.resolve(owner, selection, *role)? {
                    occurrences.push(occurrence);
                }
            }
        }
        let valid = occurrences.len() == 4
            && occurrences[0].target.is_some()
            && occurrences[0].target == occurrences[2].target
            && occurrences
                .iter()
                .zip([
                    pb::EntityKind::Ui,
                    pb::EntityKind::Persona,
                    pb::EntityKind::Ui,
                    if slice_kind == pb::EntityKind::CommandSlice {
                        pb::EntityKind::Command
                    } else {
                        pb::EntityKind::ReadModel
                    },
                ])
                .all(|(occurrence, kind)| {
                    occurrence
                        .target
                        .as_ref()
                        .is_some_and(|target| target.kind == kind as i32)
                });
        let mut state = pb::CatalogResolutionState::Unresolved;
        let mut edges = Vec::new();
        if valid {
            state = pb::CatalogResolutionState::Resolved;
            for ((selection, role), occurrence) in selected.iter().zip(occurrences) {
                let target = occurrence
                    .target
                    .ok_or_else(|| Status::data_loss("stored interaction endpoint is missing"))?;
                let endpoint = match self
                    .interaction_read(&mut query, &target, branch, vis)
                    .await
                {
                    Ok(row) => row,
                    Err(error) if error.code() == Code::NotFound => None,
                    Err(error) => return Err(error),
                };
                let target_state = if endpoint.is_some() {
                    pb::CatalogResolutionState::Resolved
                } else {
                    state = pb::CatalogResolutionState::Unavailable;
                    pb::CatalogResolutionState::Unavailable
                };
                query.budget.output()?;
                edges.push(pb::HumanInteractionCheckedEdge {
                    role: *role as i32,
                    selection: Some((*selection).clone()),
                    field: occurrence.path,
                    target: Some(target),
                    state: target_state as i32,
                    checked_target: endpoint
                        .as_ref()
                        .map(super::decision_document_queries::checked)
                        .transpose()?,
                });
            }
        }
        let lens = self.lens(vis).await?;
        let mut visited: Vec<_> = query
            .rows
            .values()
            .filter_map(Option::as_ref)
            .map(super::decision_document_queries::checked)
            .collect::<Result<_, _>>()?;
        visited.sort_by_key(Message::encode_to_vec);
        for _ in &visited {
            query.budget.output()?;
        }
        let authorized_namespaces: Vec<_> = match lens {
            Lens::Everything => visited
                .iter()
                .filter_map(|row| {
                    row.owner
                        .as_ref()?
                        .id
                        .as_ref()
                        .map(|id| id.namespace.clone())
                })
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect(),
            Lens::Only(namespaces) => namespaces
                .iter()
                .map(|namespace| namespace.as_str().to_owned())
                .collect(),
        };
        let mut digest = Sha256::new();
        for bytes in [
            request.encode_to_vec(),
            branch.unwrap_or_default().as_bytes().to_vec(),
            vis.principal().as_bytes().to_vec(),
            serde_json::to_vec(&authorized_namespaces)
                .map_err(|_| Status::internal("cannot encode checked scope"))?,
        ]
        .into_iter()
        .chain(visited.iter().map(Message::encode_to_vec))
        {
            digest.update((bytes.len() as u64).to_le_bytes());
            digest.update(bytes);
        }
        let encoded = digest
            .finalize()
            .iter()
            .fold(String::new(), |mut text, byte| {
                const HEX: &[u8; 16] = b"0123456789abcdef";
                text.push(char::from(HEX[usize::from(byte >> 4)]));
                text.push(char::from(HEX[usize::from(byte & 15)]));
                text
            });
        let revision = format!("human-interaction-read-set-v1:{encoded}");
        Ok(pb::InspectHumanInteractionResponse {
            request: Some(request),
            state: state as i32,
            edges,
            scope: Some(pb::CatalogCheckedScope {
                branch: branch.map(str::to_owned),
                authorized_namespaces,
                revision,
                visited,
                ..Default::default()
            }),
            consistency: pb::HumanInteractionConsistency::PerOwnerCheckedReadSet as i32,
        })
    }
}
