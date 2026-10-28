use std::collections::HashMap;

use pb::CatalogResolutionState as State;
use tonic::Status;
use trogon_atlas_core::design_topology::{DEFAULT_TRAVERSAL_DEPTH, MAX_TRAVERSAL_DEPTH};

use super::{
    decision_document_queries::{checked, exact, CatalogQuery, SubjectFamily},
    entity_id, entity_kind, pb, EventModelServiceImpl, StoredEntity, Visibility,
};

type Selection = pb::topology_step_target::Selection;

fn graph_ref(reference: &pb::DesignGraphRef) -> Result<pb::EntityRef, Status> {
    let result = pb::EntityRef {
        kind: pb::EntityKind::DesignGraph as i32,
        id: reference.id.clone(),
    };
    exact(&result)?;
    Ok(result)
}

fn graph_of(row: &StoredEntity) -> Option<&pb::DesignGraph> {
    trogon_atlas_core::design_topology::graph(&row.entity)
}

fn target_reference(selection: &Selection) -> Option<pb::EntityRef> {
    let (kind, id) = match selection {
        Selection::Component(r) => (pb::EntityKind::Component, r.id.as_ref()),
        Selection::Channel(r) => (pb::EntityKind::Channel, r.id.as_ref()),
        Selection::DataStore(r) => (pb::EntityKind::DataStore, r.id.as_ref()),
        Selection::DataProduct(r) => (pb::EntityKind::DataProduct, r.id.as_ref()),
        Selection::Subflow(r) => (pb::EntityKind::DesignGraph, r.id.as_ref()),
        _ => return None,
    };
    Some(pb::EntityRef {
        kind: kind as i32,
        id: id.cloned(),
    })
}

fn failure(field: &str, message: &str) -> pb::CatalogQueryFailure {
    pb::CatalogQueryFailure {
        field: field.into(),
        code: tonic::Code::FailedPrecondition as i32,
        message: message.into(),
        ..Default::default()
    }
}

struct Frame {
    row: StoredEntity,
    depth: u32,
    path: Vec<pb::EntityKey>,
}

#[derive(Default)]
struct Traversal {
    nodes: Vec<pb::TopologyTraversalNode>,
    edges: Vec<pb::TopologyTraversalEdge>,
    subflows: Vec<pb::TopologySubflowMarker>,
    cycles: Vec<pb::TopologyCycle>,
    failures: Vec<pb::CatalogQueryFailure>,
    incomplete: bool,
}

fn transition_cycles(
    owner: &pb::CatalogCheckedOwner,
    graph: &pb::DesignGraph,
    query: &mut CatalogQuery,
    out: &mut Vec<pb::TopologyCycle>,
) -> Result<(), Status> {
    #[derive(Clone, Copy, PartialEq)]
    enum Mark {
        Open,
        Done,
    }
    fn visit(
        index: usize,
        graph: &pb::DesignGraph,
        positions: &HashMap<&pb::LocalValueId, usize>,
        marks: &mut HashMap<usize, Mark>,
        stack: &mut Vec<usize>,
        found: &mut Vec<(Vec<usize>, Option<pb::ConnectionId>)>,
    ) {
        marks.insert(index, Mark::Open);
        stack.push(index);
        for transition in &graph.steps[index].transitions {
            let Some(next) = transition
                .target_step_id
                .as_ref()
                .and_then(|id| positions.get(id).copied())
            else {
                continue;
            };
            match marks.get(&next) {
                None => visit(next, graph, positions, marks, stack, found),
                Some(Mark::Open) => {
                    let start = stack.iter().position(|v| *v == next).unwrap_or(0);
                    found.push((stack[start..].to_vec(), transition.connection_id.clone()));
                }
                Some(Mark::Done) => {}
            }
        }
        stack.pop();
        marks.insert(index, Mark::Done);
    }
    let positions: HashMap<_, _> = graph
        .steps
        .iter()
        .enumerate()
        .filter_map(|(index, step)| Some((step.step_id.as_ref()?, index)))
        .collect();
    let mut marks = HashMap::new();
    let mut found = Vec::new();
    for index in 0..graph.steps.len() {
        if !marks.contains_key(&index) {
            visit(
                index,
                graph,
                &positions,
                &mut marks,
                &mut Vec::new(),
                &mut found,
            );
        }
    }
    for (path, closing) in found {
        query.budget.output()?;
        out.push(pb::TopologyCycle {
            scope: pb::topology_cycle::Scope::Transition as i32,
            graph: Some(owner.clone()),
            step_path: path
                .into_iter()
                .filter_map(|index| graph.steps[index].step_id.clone())
                .collect(),
            graph_path: Vec::new(),
            closing_connection_id: closing,
        });
    }
    Ok(())
}

impl EventModelServiceImpl {
    async fn topology_target_state(
        &self,
        query: &mut CatalogQuery,
        target: &pb::TopologyStepTarget,
    ) -> Result<(State, Option<pb::CatalogCheckedOwner>), Status> {
        let Some(selection) = target.selection.as_ref() else {
            return Ok((State::Unresolved, None));
        };
        match selection {
            Selection::CatalogMessage(subject) => {
                Ok((self.catalog_subject_state(query, subject).await?, None))
            }
            Selection::Actor(_) | Selection::Custom(_) | Selection::ExternalSystem(_) => {
                Ok((State::Resolved, None))
            }
            _ => {
                let Some(reference) = target_reference(selection) else {
                    return Ok((State::Unresolved, None));
                };
                let Ok((_, id)) = exact(&reference) else {
                    return Ok((State::Unresolved, None));
                };
                if !query.lens.admits(&id.namespace) {
                    return Ok((State::Unavailable, None));
                }
                match self.catalog_read(query, &reference).await? {
                    Some(row) => Ok((State::Resolved, Some(checked(&row)?))),
                    None => Ok((State::Unavailable, None)),
                }
            }
        }
    }

    pub(super) async fn design_graph_detail(
        &self,
        req: pb::GetDesignGraphDetailRequest,
        branch: Option<&str>,
        vis: &Visibility,
    ) -> Result<pb::GetDesignGraphDetailResponse, Status> {
        let reference = graph_ref(
            &req.graph
                .ok_or_else(|| Status::invalid_argument("graph is required"))?,
        )?;
        let mut query = self.catalog_query(branch, vis, false).await?;
        let row = self
            .catalog_read(&mut query, &reference)
            .await?
            .ok_or_else(|| Status::not_found("Design graph not found"))?;
        let graph = graph_of(&row).ok_or_else(|| Status::not_found("Design graph not found"))?;
        let declarations = self.catalog_declarations(&mut query, &row).await?;
        let mut targets = Vec::new();
        for (index, step) in graph.steps.iter().enumerate() {
            query.budget.examine(1)?;
            let Some(target) = &step.target else { continue };
            let (state, owner) = self.topology_target_state(&mut query, target).await?;
            query.budget.output()?;
            targets.push(pb::TopologyTargetState {
                step_id: step.step_id.clone(),
                field: format!("steps[{index}].target"),
                state: state as i32,
                target: Some(target.clone()),
                owner,
            });
        }
        let layout_state = if let Some(layout) = &graph.layout_document {
            let reference = pb::EntityRef {
                kind: pb::EntityKind::Document as i32,
                id: layout.id.clone(),
            };
            match exact(&reference) {
                Err(_) => State::Unresolved,
                Ok((_, id)) if !query.lens.admits(&id.namespace) => State::Unavailable,
                Ok(_) => {
                    if self.catalog_read(&mut query, &reference).await?.is_some() {
                        State::Resolved
                    } else {
                        State::Unavailable
                    }
                }
            }
        } else {
            State::Unspecified
        };
        Ok(pb::GetDesignGraphDetailResponse {
            entity: Some(row.entity),
            etag: row.etag,
            targets,
            declarations,
            layout_state: layout_state as i32,
            scope: Some(query.scope()?),
            incomplete: false,
            failures: Vec::new(),
        })
    }

    pub(super) async fn subject_design_graphs(
        &self,
        req: pb::ListSubjectDesignGraphsRequest,
        branch: Option<&str>,
        vis: &Visibility,
    ) -> Result<pb::ListSubjectDesignGraphsResponse, Status> {
        let subject = req
            .subject
            .ok_or_else(|| Status::invalid_argument("subject is required"))?;
        let (declarations, scope, state) = self
            .catalog_subject_matches(&subject, None, SubjectFamily::DesignGraphs, branch, vis)
            .await?;
        Ok(pb::ListSubjectDesignGraphsResponse {
            declarations,
            scope: Some(scope),
            incomplete: false,
            failures: Vec::new(),
            subject_state: state as i32,
        })
    }

    pub(super) async fn traverse_topology(
        &self,
        req: pb::TraverseDesignGraphTopologyRequest,
        branch: Option<&str>,
        vis: &Visibility,
    ) -> Result<pb::TraverseDesignGraphTopologyResponse, Status> {
        let reference = graph_ref(
            &req.root
                .ok_or_else(|| Status::invalid_argument("root is required"))?,
        )?;
        let max_depth = match req.max_depth {
            0 => DEFAULT_TRAVERSAL_DEPTH,
            value => value.min(MAX_TRAVERSAL_DEPTH),
        };
        let mut query = self.catalog_query(branch, vis, false).await?;
        let root = self
            .catalog_read(&mut query, &reference)
            .await?
            .ok_or_else(|| Status::not_found("Design graph not found"))?;
        if graph_of(&root).is_none() {
            return Err(Status::not_found("Design graph not found"));
        }
        let mut out = Traversal::default();
        let mut pending = vec![Frame {
            path: vec![pb::EntityKey::new(
                entity_kind(&root.entity)?,
                entity_id(&root.entity)?,
            )],
            row: root,
            depth: 0,
        }];
        while let Some(frame) = pending.pop() {
            let children = self
                .expand_graph(&mut query, &frame, req.follow_subflows, max_depth, &mut out)
                .await?;
            pending.extend(children.into_iter().rev());
        }
        Ok(pb::TraverseDesignGraphTopologyResponse {
            nodes: out.nodes,
            edges: out.edges,
            subflows: out.subflows,
            cycles: out.cycles,
            scope: Some(query.scope()?),
            incomplete: out.incomplete,
            failures: out.failures,
            applied_max_depth: max_depth,
            follow_subflows: req.follow_subflows,
        })
    }

    async fn expand_graph(
        &self,
        query: &mut CatalogQuery,
        frame: &Frame,
        follow: bool,
        max_depth: u32,
        out: &mut Traversal,
    ) -> Result<Vec<Frame>, Status> {
        let Some(graph) = graph_of(&frame.row) else {
            return Ok(Vec::new());
        };
        let owner = checked(&frame.row)?;
        let mut children = Vec::new();
        for (index, step) in graph.steps.iter().enumerate() {
            query.budget.examine(1)?;
            let (state, _) = match &step.target {
                Some(target) => self.topology_target_state(query, target).await?,
                None => (State::Unspecified, None),
            };
            query.budget.output()?;
            out.nodes.push(pb::TopologyTraversalNode {
                graph: Some(owner.clone()),
                step_id: step.step_id.clone(),
                depth: frame.depth,
                title: step.title.clone(),
                summary: step.summary.clone(),
                target: step.target.clone(),
                target_state: state as i32,
                authored_entry: step
                    .step_id
                    .as_ref()
                    .is_some_and(|id| graph.entry_steps.contains(id)),
            });
            for transition in &step.transitions {
                query.budget.examine(1)?;
                query.budget.output()?;
                out.edges.push(pb::TopologyTraversalEdge {
                    kind: pb::topology_traversal_edge::Kind::Transition as i32,
                    graph: Some(owner.clone()),
                    from_step_id: step.step_id.clone(),
                    connection_id: transition.connection_id.clone(),
                    label: transition.label.clone(),
                    to_step_id: transition.target_step_id.clone(),
                    to_graph: None,
                });
            }
            let Some(Selection::Subflow(subflow)) =
                step.target.as_ref().and_then(|t| t.selection.as_ref())
            else {
                continue;
            };
            let field = format!("steps[{index}].target.subflow");
            let marker = |state: pb::TopologySubflowState| pb::TopologySubflowMarker {
                graph: Some(owner.clone()),
                step_id: step.step_id.clone(),
                subflow: Some(subflow.clone()),
                state: state as i32,
                depth: frame.depth,
            };
            query.budget.output()?;
            if !follow {
                out.subflows
                    .push(marker(pb::TopologySubflowState::Unfollowed));
                continue;
            }
            let child = match graph_ref(subflow) {
                Ok(reference) if query.lens.admits(&exact(&reference)?.1.namespace) => self
                    .catalog_read(query, &reference)
                    .await?
                    .filter(|row| graph_of(row).is_some()),
                _ => None,
            };
            let Some(child) = child else {
                out.incomplete = true;
                out.failures
                    .push(failure(&field, "Subflow graph is unavailable"));
                out.subflows
                    .push(marker(pb::TopologySubflowState::Unavailable));
                continue;
            };
            let key = pb::EntityKey::new(entity_kind(&child.entity)?, entity_id(&child.entity)?);
            if frame.path.contains(&key) {
                let mut graph_path: Vec<_> = frame
                    .path
                    .iter()
                    .map(|key| pb::DesignGraphRef { id: Some(key.id()) })
                    .collect();
                graph_path.push(subflow.clone());
                query.budget.output()?;
                out.cycles.push(pb::TopologyCycle {
                    scope: pb::topology_cycle::Scope::Subflow as i32,
                    graph: Some(owner.clone()),
                    step_path: step.step_id.clone().into_iter().collect(),
                    graph_path,
                    closing_connection_id: None,
                });
                out.subflows.push(marker(pb::TopologySubflowState::Cycle));
                continue;
            }
            if frame.depth + 1 > max_depth {
                out.incomplete = true;
                out.failures.push(failure(
                    &field,
                    "Depth limit reached before expanding this subflow",
                ));
                out.subflows
                    .push(marker(pb::TopologySubflowState::DepthLimit));
                continue;
            }
            query.budget.output()?;
            out.edges.push(pb::TopologyTraversalEdge {
                kind: pb::topology_traversal_edge::Kind::Subflow as i32,
                graph: Some(owner.clone()),
                from_step_id: step.step_id.clone(),
                to_graph: Some(subflow.clone()),
                ..Default::default()
            });
            out.subflows
                .push(marker(pb::TopologySubflowState::Expanded));
            let mut path = frame.path.clone();
            path.push(key);
            children.push(Frame {
                row: child,
                depth: frame.depth + 1,
                path,
            });
        }
        transition_cycles(&owner, graph, query, &mut out.cycles)?;
        Ok(children)
    }

    pub(super) async fn incoming_subflows(
        &self,
        req: pb::ListIncomingDesignGraphSubflowsRequest,
        branch: Option<&str>,
        vis: &Visibility,
    ) -> Result<pb::ListIncomingDesignGraphSubflowsResponse, Status> {
        let selection = req
            .graph
            .ok_or_else(|| Status::invalid_argument("graph is required"))?;
        let reference = graph_ref(&selection)?;
        self.require_visible(vis, &exact(&reference)?.1.namespace)
            .await?;
        let mut query = self.catalog_query(branch, vis, true).await?;
        let state = if self.catalog_read(&mut query, &reference).await?.is_some() {
            State::Resolved
        } else {
            State::Unavailable
        };
        let mut references = Vec::new();
        for row in query.rows.clone() {
            let Some(graph) = graph_of(&row) else {
                continue;
            };
            let owner = checked(&row)?;
            for (index, step) in graph.steps.iter().enumerate() {
                query.budget.examine(1)?;
                if !matches!(
                    step.target.as_ref().and_then(|t| t.selection.as_ref()),
                    Some(Selection::Subflow(subflow)) if subflow == &selection
                ) {
                    continue;
                }
                let transitions = trogon_atlas_core::design_topology::transitions(graph)
                    .into_iter()
                    .filter(|edge| {
                        edge.source_step_id == step.step_id || edge.target_step_id == step.step_id
                    })
                    .map(|edge| {
                        query.budget.output()?;
                        Ok(pb::TopologyTraversalEdge {
                            kind: pb::topology_traversal_edge::Kind::Transition as i32,
                            graph: Some(owner.clone()),
                            from_step_id: edge.source_step_id,
                            connection_id: edge.connection_id,
                            label: edge.label,
                            to_step_id: edge.target_step_id,
                            to_graph: None,
                        })
                    })
                    .collect::<Result<Vec<_>, Status>>()?;
                query.budget.output()?;
                references.push(pb::DesignGraphSubflowReference {
                    owner: Some(owner.clone()),
                    field: format!("steps[{index}].target.subflow"),
                    state: state as i32,
                    step_id: step.step_id.clone(),
                    subflow: Some(selection.clone()),
                    transitions,
                });
            }
        }
        Ok(pb::ListIncomingDesignGraphSubflowsResponse {
            references,
            scope: Some(query.scope()?),
            incomplete: false,
            failures: Vec::new(),
            graph_state: state as i32,
        })
    }
}
