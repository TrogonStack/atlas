use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};

use pb::DataProductSelectionState as State;
use sha2::{Digest, Sha256};
use tonic::{Code, Status};

use super::{
    entity_id, entity_kind, pb, store_err, EventModelServiceImpl, Lens, StoredEntity, Visibility,
};
use crate::validation::architecture_resources::ArchitectureContext;

type World = HashMap<pb::EntityKey, StoredEntity>;
type PathEdges = HashMap<pb::EntityKey, Vec<(pb::EntityKey, Vec<pb::ConnectionRef>)>>;

impl EventModelServiceImpl {
    pub(super) async fn inspect_product(
        &self,
        product: &pb::DataProductRef,
        branch: Option<&str>,
        vis: &Visibility,
    ) -> Result<pb::InspectDataProductResponse, Status> {
        let id = super::require_id(product.id.as_ref(), "product")?;
        self.require_visible(vis, &id.namespace).await?;
        if let Some(branch) = branch {
            self.require_branch(branch).await?;
        }
        let _guard = self.lock_mutations().await;
        let stored = self
            .store
            .get(pb::EntityKind::DataProduct, id, branch)
            .await
            .map_err(store_err)?;
        let Some(pb::entity::Kind::DataProduct(authored)) = &stored.entity.kind else {
            return Err(Status::internal("Data product has an invalid stored kind"));
        };
        let lens = self.lens(vis).await?;
        let mut world = World::new();
        insert(&mut world, stored.clone())?;
        for row in self
            .architecture_validation_context(std::slice::from_ref(&stored), branch, vis)
            .await?
        {
            insert(&mut world, row)?;
        }
        let mut incomplete = false;
        let mut emissions = Vec::new();
        let mut event_states = HashMap::new();
        if super::data_product_events::inputs(authored)
            .next()
            .is_some()
        {
            let proposal = super::architecture_admission::ArchitectureProposal {
                overlay: HashMap::new(),
                landing: None,
            };
            let (rows, complete) = self
                .data_product_emission_rows(branch, vis, &proposal)
                .await?;
            let mut budget = super::data_product_events::EventConfirmation::default();
            for (index, event) in super::data_product_events::inputs(authored) {
                if !world.contains_key(&pb::EntityKey::new(pb::EntityKind::Event, event)) {
                    event_states.insert(index, State::Unresolved);
                    continue;
                }
                match budget.confirm(event, &rows, complete) {
                    Ok(witnesses) => {
                        event_states.insert(
                            index,
                            if witnesses.is_empty() {
                                State::Unresolved
                            } else {
                                State::Resolved
                            },
                        );
                        for witness in witnesses {
                            emissions.push(pb::DataProductEmissionEvidence {
                                input: Some(port_ref(
                                    &stored.entity,
                                    authored.inputs[index].port_id.clone(),
                                    pb::DataPortDirection::Input,
                                )),
                                event: Some(pb::EventRef {
                                    id: Some(event.clone()),
                                }),
                                emission: Some(witness),
                            });
                        }
                    }
                    Err(status) if status.code() == Code::ResourceExhausted => {
                        incomplete = true;
                        event_states.insert(index, State::Unknown);
                    }
                    Err(status) => return Err(status),
                }
            }
            for row in rows {
                insert(&mut world, row)?;
            }
        }
        let namespaces: Vec<_> = match &lens {
            Lens::Everything => Vec::new(),
            Lens::Only(values) => values.iter().map(|id| id.as_str().to_owned()).collect(),
        };
        let mut behavior_complete = true;
        if !matches!(&lens, Lens::Only(values) if values.is_empty()) {
            let rows = self
                .store
                .list(
                    super::ListFilter {
                        kinds: &[
                            pb::EntityKind::AutomationSlice,
                            pb::EntityKind::ReadModelSlice,
                            pb::EntityKind::CommandSlice,
                        ],
                        namespaces: &namespaces,
                        ..Default::default()
                    },
                    Some(5_001),
                    branch,
                )
                .await
                .map_err(store_err)?;
            if rows.len() > 5_000 {
                behavior_complete = false;
            }
            for row in rows.into_iter().take(5_000) {
                let key = pb::EntityKey::new(entity_kind(&row.entity)?, entity_id(&row.entity)?);
                world.entry(key).or_insert(row);
            }
        }
        let targets: Vec<_> = world
            .values()
            .filter(|row| {
                matches!(
                    row.entity.kind,
                    Some(
                        pb::entity::Kind::CommandSlice(_)
                            | pb::entity::Kind::AutomationSlice(_)
                            | pb::entity::Kind::ReadModelSlice(_)
                    )
                )
            })
            .flat_map(|row| trogon_atlas_core::refs::outbound_refs(&row.entity))
            .filter_map(|r| r.to_kind.map(|kind| (kind, r.to_id)))
            .collect();
        let mut missing = HashSet::new();
        for (kind, target) in targets {
            let key = pb::EntityKey::new(kind, &target);
            if world.contains_key(&key) || !missing.insert(key.clone()) {
                continue;
            }
            if !lens.admits(&target.namespace) {
                behavior_complete = false;
                continue;
            }
            if world.len() >= 10_000 {
                behavior_complete = false;
                break;
            }
            match self.store.get(kind, &target, branch).await {
                Ok(row) => {
                    world.insert(key, row);
                }
                Err(trogon_atlas_store::error::StoreError::NotFound) => {
                    behavior_complete = false;
                }
                Err(error) => return Err(store_err(error)),
            }
        }
        let context = ArchitectureContext::new(world.values());
        let issues = crate::data_product_resources::validate_in_context(&stored.entity, &context);
        let mut compiled = HashMap::new();
        for selection in crate::data_product_resources::library_types(authored) {
            let (Some(id), Some(message)) = (
                selection.library.as_ref().and_then(|r| r.id.as_ref()),
                &selection.message,
            ) else {
                continue;
            };
            let key = (
                pb::EntityKey::new(pb::EntityKind::TypeLibrary, id),
                message.value.clone(),
            );
            if let std::collections::hash_map::Entry::Vacant(entry) = compiled.entry(key) {
                let rows: Vec<_> = world.values().cloned().collect();
                let state = match self
                    .type_libraries
                    .check_contract_type(id, &message.value, &rows)
                    .await
                {
                    Ok(()) => State::Resolved,
                    Err(error)
                        if matches!(
                            error.code(),
                            Code::InvalidArgument | Code::FailedPrecondition
                        ) =>
                    {
                        State::Unresolved
                    }
                    Err(error) => return Err(error),
                };
                entry.insert(state);
            }
        }
        let mut ports = Vec::new();
        for (index, port) in authored.inputs.iter().enumerate() {
            let data = match port.data.as_ref().and_then(|v| v.target.as_ref()) {
                None if port.data.is_none() => State::Undeclared,
                Some(pb::input_data_selection::Target::ReadModel(r)) => {
                    selection_state(pb::EntityKind::ReadModel, r.id.as_ref(), &world)
                }
                Some(pb::input_data_selection::Target::Event(_)) => event_states
                    .get(&index)
                    .copied()
                    .unwrap_or(State::Unresolved),
                _ => State::Unresolved,
            };
            ports.push(pb::DataProductPortState {
                port: Some(port_ref(
                    &stored.entity,
                    port.port_id.clone(),
                    pb::DataPortDirection::Input,
                )),
                data: data as i32,
                incomplete: data == State::Unknown,
                ..Default::default()
            });
        }
        for port in &authored.outputs {
            let target = port
                .data
                .as_ref()
                .and_then(|d| d.read_model.as_ref())
                .and_then(|r| r.id.as_ref());
            let data = if port.data.is_none() {
                State::Undeclared
            } else {
                selection_state(pb::EntityKind::ReadModel, target, &world)
            };
            let contract = match port.contract.as_ref().and_then(|c| c.r#type.as_ref()) {
                None if port.contract.is_none() => State::Undeclared,
                Some(_)
                    if !crate::data_product_resources::contract_resolves(
                        port.contract.as_ref(),
                        &context,
                    ) =>
                {
                    State::Unresolved
                }
                Some(pb::output_contract::Type::ContractType(_)) => State::Opaque,
                Some(pb::output_contract::Type::LibraryType(r)) => r
                    .library
                    .as_ref()
                    .and_then(|r| r.id.as_ref())
                    .zip(r.message.as_ref())
                    .and_then(|(id, m)| {
                        compiled.get(&(
                            pb::EntityKey::new(pb::EntityKind::TypeLibrary, id),
                            m.value.clone(),
                        ))
                    })
                    .copied()
                    .unwrap_or(State::Unresolved),
                Some(_) => State::Resolved,
                None => State::Unresolved,
            };
            let access = port
                .access
                .iter()
                .map(|declaration| {
                    let valid = crate::data_product_resources::access_resolves(
                        declaration,
                        target,
                        &context,
                    );
                    let provider = match &declaration.selection {
                        Some(pb::output_access::Selection::Operation(value))
                            if value.provider_exposure.is_none() =>
                        {
                            State::Undeclared
                        }
                        Some(pb::output_access::Selection::Operation(_)) => {
                            if valid {
                                State::Resolved
                            } else {
                                State::Unresolved
                            }
                        }
                        _ => State::Unknown,
                    };
                    let checked_connections = if valid {
                        access_connections(declaration)
                    } else {
                        Vec::new()
                    };
                    pb::DataProductAccessState {
                        declaration: Some(redact_access(declaration, &lens)),
                        state: if valid {
                            State::Resolved
                        } else {
                            State::Unresolved
                        } as i32,
                        provider: provider as i32,
                        checked_connections,
                    }
                })
                .collect();
            let lineage = match &port.derived_from {
                None => State::Unknown,
                Some(value) if value.inputs.is_empty() => State::ExplicitEmpty,
                Some(value)
                    if value
                        .inputs
                        .iter()
                        .all(|r| authored.inputs.iter().any(|p| p.port_id == r.input_port_id)) =>
                {
                    State::Declared
                }
                Some(_) => State::Unresolved,
            };
            ports.push(pb::DataProductPortState {
                port: Some(port_ref(
                    &stored.entity,
                    port.port_id.clone(),
                    pb::DataPortDirection::Output,
                )),
                data: data as i32,
                contract: contract as i32,
                lineage: lineage as i32,
                access,
                incomplete: !behavior_complete,
            });
        }
        let (edges, edge_complete) = producing_edges(&world);
        behavior_complete &= edge_complete;
        let mut producing = Vec::new();
        let mut evidence_work = 0;
        let targets: HashSet<_> = authored
            .outputs
            .iter()
            .filter_map(|output| {
                output
                    .data
                    .as_ref()
                    .and_then(|data| data.read_model.as_ref())
                    .and_then(|reference| reference.id.as_ref())
                    .map(|id| pb::EntityKey::new(pb::EntityKind::ReadModel, id))
            })
            .collect();
        for (index, input) in authored.inputs.iter().enumerate() {
            if !behavior_complete {
                break;
            }
            if ports[index].data != State::Resolved as i32 {
                continue;
            }
            let start = match input.data.as_ref().and_then(|d| d.target.as_ref()) {
                Some(pb::input_data_selection::Target::Event(r)) => {
                    r.id.as_ref()
                        .map(|id| pb::EntityKey::new(pb::EntityKind::Event, id))
                }
                Some(pb::input_data_selection::Target::ReadModel(r)) => {
                    r.id.as_ref()
                        .map(|id| pb::EntityKey::new(pb::EntityKind::ReadModel, id))
                }
                None => None,
            };
            let Some(start) = start else { continue };
            let (paths, complete) = paths_from(start, &edges, &targets);
            behavior_complete &= complete;
            for (output_index, output) in authored.outputs.iter().enumerate() {
                if ports[authored.inputs.len() + output_index].data != State::Resolved as i32 {
                    continue;
                }
                if let Some(path) = output
                    .data
                    .as_ref()
                    .and_then(|d| d.read_model.as_ref())
                    .and_then(|r| r.id.as_ref())
                    .and_then(|id| paths.get(&pb::EntityKey::new(pb::EntityKind::ReadModel, id)))
                {
                    if !path.is_empty() {
                        evidence_work += path.len();
                        if evidence_work > 5_000 {
                            behavior_complete = false;
                            break;
                        }
                        producing.push(pb::DataProductProducingEvidence {
                            input: Some(port_ref(
                                &stored.entity,
                                input.port_id.clone(),
                                pb::DataPortDirection::Input,
                            )),
                            output: Some(port_ref(
                                &stored.entity,
                                output.port_id.clone(),
                                pb::DataPortDirection::Output,
                            )),
                            path: path.clone(),
                        });
                    }
                }
            }
        }
        if !behavior_complete {
            producing.clear();
        }
        incomplete |= !behavior_complete;
        for port in &mut ports {
            port.incomplete |= !behavior_complete;
        }
        let unresolved = !issues.is_empty()
            || ports.iter().any(|port| {
                [port.data, port.contract, port.lineage].contains(&(State::Unresolved as i32))
                    || port
                        .access
                        .iter()
                        .any(|a| a.state == State::Unresolved as i32)
            });
        let offering = if unresolved {
            pb::DataProductOfferingState::Unresolved
        } else if ports.iter().any(|p| p.data == State::Unknown as i32) {
            pb::DataProductOfferingState::Unknown
        } else if authored.outputs.iter().any(|p| p.data.is_none()) {
            pb::DataProductOfferingState::Draft
        } else {
            pb::DataProductOfferingState::Mapped
        };
        let scope = checked_scope(&world, branch, &lens, incomplete)?;
        Ok(pb::InspectDataProductResponse {
            product: Some(redact_product(authored, &lens)),
            etag: stored.etag,
            scope: Some(scope),
            ports,
            emissions,
            producing,
            incomplete,
            offering: offering as i32,
        })
    }
}

fn insert(world: &mut World, stored: StoredEntity) -> Result<(), Status> {
    world.insert(
        pb::EntityKey::new(entity_kind(&stored.entity)?, entity_id(&stored.entity)?),
        stored,
    );
    Ok(())
}
fn port_ref(
    entity: &pb::Entity,
    id: Option<pb::DataPortId>,
    direction: pb::DataPortDirection,
) -> pb::DataPortRef {
    pb::DataPortRef {
        product: crate::conv::entity_id(entity)
            .ok()
            .cloned()
            .map(|id| pb::DataProductRef { id: Some(id) }),
        owner_uid: entity
            .system
            .as_ref()
            .map_or_else(String::new, |s| s.uid.clone()),
        direction: direction as i32,
        port_id: id,
    }
}
fn selection_state(kind: pb::EntityKind, id: Option<&pb::Id>, world: &World) -> State {
    if id.is_some_and(|id| world.contains_key(&pb::EntityKey::new(kind, id))) {
        State::Resolved
    } else {
        State::Unresolved
    }
}
fn access_connections(access: &pb::OutputAccess) -> Vec<pb::ConnectionRef> {
    match &access.selection {
        Some(pb::output_access::Selection::Store(s)) => s.attachment.iter().cloned().collect(),
        Some(pb::output_access::Selection::Channel(s)) => s
            .binding
            .iter()
            .chain(s.observation.iter())
            .cloned()
            .collect(),
        Some(pb::output_access::Selection::Operation(s)) => s
            .response
            .iter()
            .chain(s.provider_exposure.iter())
            .cloned()
            .collect(),
        None => Vec::new(),
    }
}
fn checked_scope(
    world: &World,
    branch: Option<&str>,
    lens: &Lens,
    incomplete: bool,
) -> Result<pb::DataProductCheckedScope, Status> {
    let mut rows: Vec<_> = world.iter().collect();
    rows.sort_by_key(|(key, _)| (key.kind as i32, &key.namespace, &key.slug, key.version));
    let namespaces: BTreeSet<_> = match lens {
        Lens::Everything => world.keys().map(|key| key.namespace.clone()).collect(),
        Lens::Only(values) => values.iter().map(|id| id.as_str().to_owned()).collect(),
    };
    let mut hash = Sha256::new();
    hash.update(
        serde_json::to_vec(&(
            branch,
            &namespaces,
            incomplete,
            matches!(lens, Lens::Everything),
        ))
        .map_err(|_| Status::internal("Cannot encode checked context"))?,
    );
    let mut visited = Vec::new();
    for (_, row) in rows {
        let content = trogon_atlas_core::entity_content_hash(&row.entity)
            .map_err(|_| {
                Status::failed_precondition("Checked entity content cannot be identified")
            })?
            .to_hex();
        hash.update((content.len() as u64).to_le_bytes());
        hash.update(content.as_bytes());
        hash.update((row.etag.len() as u64).to_le_bytes());
        hash.update(row.etag.as_bytes());
        let uid = row
            .entity
            .system
            .as_ref()
            .map_or("", |system| system.uid.as_str());
        hash.update((uid.len() as u64).to_le_bytes());
        hash.update(uid.as_bytes());
        visited.push(crate::conv::entity_to_ref(&row.entity)?);
    }
    Ok(pb::DataProductCheckedScope {
        branch: branch.map(str::to_owned),
        authorized_namespaces: namespaces.into_iter().collect(),
        revision: format!(
            "checked-v1:{}",
            hash.finalize()
                .iter()
                .fold(String::new(), |mut value, byte| {
                    const HEX: &[u8; 16] = b"0123456789abcdef";
                    value.push(char::from(HEX[usize::from(byte >> 4)]));
                    value.push(char::from(HEX[usize::from(byte & 15)]));
                    value
                })
        ),
        visited,
    })
}

fn redact_access(access: &pb::OutputAccess, lens: &Lens) -> pb::OutputAccess {
    let product = pb::DataProduct {
        outputs: vec![pb::DataOutputPort {
            access: vec![access.clone()],
            ..Default::default()
        }],
        ..Default::default()
    };
    let mut access = access.clone();
    if has_hidden_refs(&product, lens) {
        access.selection = None;
    }
    access
}
fn has_hidden_refs(product: &pb::DataProduct, lens: &Lens) -> bool {
    trogon_atlas_core::refs::outbound_refs(&pb::Entity {
        kind: Some(pb::entity::Kind::DataProduct(product.clone())),
        ..Default::default()
    })
    .iter()
    .any(|r| !lens.admits(&r.to_id.namespace))
}
fn redact_product(product: &pb::DataProduct, lens: &Lens) -> pb::DataProduct {
    let mut product = product.clone();
    if product
        .supersedes
        .as_ref()
        .is_some_and(|id| !lens.admits(&id.namespace))
    {
        product.supersedes = None;
    }
    for input in &mut product.inputs {
        if let Some(data) = &mut input.data {
            let id = match &data.target {
                Some(pb::input_data_selection::Target::ReadModel(r)) => r.id.as_ref(),
                Some(pb::input_data_selection::Target::Event(r)) => r.id.as_ref(),
                None => None,
            };
            if id.is_some_and(|id| !lens.admits(&id.namespace)) {
                data.target = None;
            }
        }
    }
    for output in &mut product.outputs {
        if let Some(data) = &mut output.data {
            if data
                .read_model
                .as_ref()
                .and_then(|r| r.id.as_ref())
                .is_some_and(|id| !lens.admits(&id.namespace))
            {
                data.read_model = None;
            }
        }
        if let Some(contract) = &mut output.contract {
            let selected = pb::DataProduct {
                outputs: vec![pb::DataOutputPort {
                    contract: Some(contract.clone()),
                    ..Default::default()
                }],
                ..Default::default()
            };
            if has_hidden_refs(&selected, lens) {
                contract.r#type = None;
            }
        }
        for access in &mut output.access {
            *access = redact_access(access, lens);
        }
    }
    product
}

fn connection(entity: &pb::Entity, id: Option<&pb::ConnectionId>) -> Option<pb::ConnectionRef> {
    let reference = pb::ConnectionRef {
        owner: crate::conv::entity_to_ref(entity).ok(),
        owner_uid: entity.system.as_ref()?.uid.clone(),
        id: id.cloned(),
    };
    reference.id.as_ref()?;
    Some(reference)
}
fn producing_edges(world: &World) -> (PathEdges, bool) {
    let mut edges = PathEdges::new();
    let mut complete = true;
    let mut examined = 0;
    let key = |kind, id: Option<&pb::Id>| {
        id.map(|id| pb::EntityKey::new(kind, id))
            .filter(|key| world.contains_key(key))
    };
    let mut rows: Vec<_> = world.iter().collect();
    rows.sort_by_key(|(key, _)| (key.kind as i32, &key.namespace, &key.slug, key.version));
    for (_, stored) in rows {
        let entity = &stored.entity;
        let work = match &entity.kind {
            Some(pb::entity::Kind::CommandSlice(slice)) => slice.emitted_events.len(),
            Some(pb::entity::Kind::ReadModelSlice(slice)) => slice.source_events.len(),
            Some(pb::entity::Kind::AutomationSlice(slice)) => slice.source_read_models.len(),
            _ => continue,
        };
        examined += work;
        if examined > 5_000 {
            complete = false;
            break;
        }
        if trogon_atlas_core::connections::validate_stored_connection_ids(entity).is_err() {
            complete = false;
            continue;
        }
        let mut add = |from: Option<pb::EntityKey>,
                       to: Option<pb::EntityKey>,
                       path: Vec<Option<pb::ConnectionRef>>| {
            if let (Some(from), Some(to), Some(path)) =
                (from, to, path.into_iter().collect::<Option<Vec<_>>>())
            {
                edges.entry(from).or_default().push((to, path));
            } else {
                complete = false;
            }
        };
        match &entity.kind {
            Some(pb::entity::Kind::ReadModelSlice(slice)) => {
                for event in &slice.source_events {
                    add(
                        key(
                            pb::EntityKind::Event,
                            event.event.as_ref().and_then(|r| r.id.as_ref()),
                        ),
                        key(
                            pb::EntityKind::ReadModel,
                            slice
                                .read_model
                                .as_ref()
                                .and_then(|e| e.read_model.as_ref())
                                .and_then(|r| r.id.as_ref()),
                        ),
                        vec![
                            connection(entity, event.connection_id.as_ref()),
                            connection(
                                entity,
                                slice
                                    .read_model
                                    .as_ref()
                                    .and_then(|e| e.connection_id.as_ref()),
                            ),
                        ],
                    );
                }
            }
            Some(pb::entity::Kind::CommandSlice(slice)) => {
                for event in &slice.emitted_events {
                    add(
                        key(
                            pb::EntityKind::Command,
                            slice
                                .command
                                .as_ref()
                                .and_then(|e| e.command.as_ref())
                                .and_then(|r| r.id.as_ref()),
                        ),
                        key(
                            pb::EntityKind::Event,
                            event.event.as_ref().and_then(|r| r.id.as_ref()),
                        ),
                        vec![
                            connection(
                                entity,
                                slice
                                    .command
                                    .as_ref()
                                    .and_then(|e| e.connection_id.as_ref()),
                            ),
                            connection(entity, event.connection_id.as_ref()),
                        ],
                    );
                }
            }
            Some(pb::entity::Kind::AutomationSlice(slice)) => {
                if slice.emitted_command.is_none() {
                    continue;
                }
                if key(
                    pb::EntityKind::Processor,
                    slice
                        .processor
                        .as_ref()
                        .and_then(|e| e.processor.as_ref())
                        .and_then(|r| r.id.as_ref()),
                )
                .is_none()
                {
                    complete = false;
                    continue;
                }
                for source in &slice.source_read_models {
                    add(
                        key(
                            pb::EntityKind::ReadModel,
                            source.read_model.as_ref().and_then(|r| r.id.as_ref()),
                        ),
                        key(
                            pb::EntityKind::Command,
                            slice
                                .emitted_command
                                .as_ref()
                                .and_then(|e| e.command.as_ref())
                                .and_then(|r| r.id.as_ref()),
                        ),
                        vec![
                            connection(entity, source.connection_id.as_ref()),
                            connection(
                                entity,
                                slice
                                    .processor
                                    .as_ref()
                                    .and_then(|e| e.connection_id.as_ref()),
                            ),
                            connection(
                                entity,
                                slice
                                    .emitted_command
                                    .as_ref()
                                    .and_then(|e| e.connection_id.as_ref()),
                            ),
                        ],
                    );
                }
            }
            _ => {}
        }
    }
    (edges, complete)
}
fn paths_from(
    start: pb::EntityKey,
    edges: &PathEdges,
    targets: &HashSet<pb::EntityKey>,
) -> (HashMap<pb::EntityKey, Vec<pb::ConnectionRef>>, bool) {
    let mut parents = HashMap::from([(start.clone(), None)]);
    let mut pending = VecDeque::from([start]);
    let mut examined = 0;
    while let Some(node) = pending.pop_front() {
        if let Some(next) = edges.get(&node) {
            for (target, segment) in next {
                examined += 1;
                if examined > 5_000 {
                    return (HashMap::new(), false);
                }
                if parents.contains_key(target) {
                    continue;
                }
                parents.insert(target.clone(), Some((node.clone(), segment)));
                pending.push_back(target.clone());
            }
        }
    }
    let mut degrees: HashMap<_, usize> = parents.keys().cloned().map(|key| (key, 0)).collect();
    for node in parents.keys() {
        if let Some(next) = edges.get(node) {
            for (target, _) in next {
                if let Some(degree) = degrees.get_mut(target) {
                    *degree += 1;
                }
            }
        }
    }
    let mut roots: VecDeque<_> = degrees
        .iter()
        .filter(|(_, degree)| **degree == 0)
        .map(|(key, _)| key.clone())
        .collect();
    let mut removed = 0;
    while let Some(node) = roots.pop_front() {
        removed += 1;
        if let Some(next) = edges.get(&node) {
            for (target, _) in next {
                if let Some(degree) = degrees.get_mut(target) {
                    *degree -= 1;
                    if *degree == 0 {
                        roots.push_back(target.clone());
                    }
                }
            }
        }
    }
    if removed != parents.len() {
        return (HashMap::new(), false);
    }
    let mut paths = HashMap::new();
    let mut work = 0;
    for target in targets {
        if !parents.contains_key(target) {
            continue;
        }
        let mut current = target.clone();
        let mut segments = Vec::new();
        while let Some(Some((parent, segment))) = parents.get(&current) {
            work += segment.len();
            if work > 5_000 {
                return (HashMap::new(), false);
            }
            segments.push(segment.as_slice());
            current = parent.clone();
        }
        paths.insert(
            target.clone(),
            segments.into_iter().rev().flatten().cloned().collect(),
        );
    }
    (paths, true)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    fn key(index: usize) -> pb::EntityKey {
        pb::EntityKey::new(
            pb::EntityKind::ReadModel,
            &pb::Id {
                namespace: "paths".into(),
                slug: format!("node-{index}"),
                version: 1,
            },
        )
    }
    #[test]
    fn data_product_long_paths_only_materialize_requested_evidence_with_a_combined_bound() {
        let edges: PathEdges = (0..5_000)
            .map(|index| {
                (
                    key(index),
                    vec![(key(index + 1), vec![pb::ConnectionRef::default()])],
                )
            })
            .collect();
        let (paths, complete) = paths_from(key(0), &edges, &HashSet::from([key(5_000)]));
        assert!(complete);
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[&key(5_000)].len(), 5_000);
        let (paths, complete) =
            paths_from(key(0), &edges, &HashSet::from([key(4_999), key(5_000)]));
        assert!(!complete);
        assert_eq!(paths.len(), 0);
    }
    #[test]
    fn data_product_reachable_cycles_cannot_claim_verified_producing_paths() {
        let edges = HashMap::from([
            (key(0), vec![(key(1), vec![pb::ConnectionRef::default()])]),
            (key(1), vec![(key(0), vec![pb::ConnectionRef::default()])]),
        ]);
        let (paths, complete) = paths_from(key(0), &edges, &HashSet::from([key(1)]));
        assert!(!complete);
        assert_eq!(paths.len(), 0);
    }
}
