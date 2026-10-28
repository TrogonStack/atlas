use std::collections::{BTreeSet, HashMap, HashSet};

use pb::CatalogResolutionState as State;
use prost::Message as _;
use sha2::{Digest, Sha256};
use tonic::Status;

use super::{
    entity_id, entity_kind, pb, store_err, EventModelServiceImpl, Lens, StoredEntity, Visibility,
};
use crate::decision_document_resources::{self as resources, CatalogBudget};

struct CatalogQuery {
    world: HashMap<pb::EntityKey, Option<StoredEntity>>,
    rows: Vec<StoredEntity>,
    lens: Lens,
    branch: Option<String>,
    budget: CatalogBudget,
    subjects: Vec<pb::CatalogSubject>,
    documents: Vec<pb::DocumentSelection>,
}

fn checked(row: &StoredEntity) -> Result<pb::CatalogCheckedOwner, Status> {
    Ok(pb::CatalogCheckedOwner {
        owner: Some(crate::conv::entity_to_ref(&row.entity)?),
        uid: row
            .entity
            .system
            .as_ref()
            .map_or_else(String::new, |system| system.uid.clone()),
        etag: row.etag.clone(),
    })
}

fn exact(reference: &pb::EntityRef) -> Result<(pb::EntityKind, &pb::Id), Status> {
    let kind = super::kind_from_i32(reference.kind, "catalog.kind")?;
    let id = super::require_id(reference.id.as_ref(), "catalog.id")?;
    if id.version == 0 {
        return Err(Status::invalid_argument(
            "catalog selection requires an exact positive version",
        ));
    }
    Ok((kind, id))
}

fn decision_ref(reference: &pb::DecisionRef) -> Result<pb::EntityRef, Status> {
    let result = pb::EntityRef {
        kind: pb::EntityKind::Decision as i32,
        id: reference.id.clone(),
    };
    exact(&result)?;
    Ok(result)
}

impl CatalogQuery {
    fn scope(&self) -> Result<pb::CatalogCheckedScope, Status> {
        let namespaces: BTreeSet<_> = match &self.lens {
            Lens::Everything => self.world.keys().map(|key| key.namespace.clone()).collect(),
            Lens::Only(namespaces) => namespaces
                .iter()
                .map(|namespace| namespace.as_str().to_owned())
                .collect(),
        };
        let mut rows: Vec<_> = self.world.iter().collect();
        rows.sort_by_key(|(key, _)| (key.kind as i32, &key.namespace, &key.slug, key.version));
        let mut hash = Sha256::new();
        let mut feed = |bytes: &[u8]| {
            hash.update((bytes.len() as u64).to_le_bytes());
            hash.update(bytes);
        };
        feed(
            &serde_json::to_vec(&(
                &self.branch,
                &namespaces,
                matches!(self.lens, Lens::Everything),
            ))
            .map_err(|_| Status::internal("cannot encode catalog scope"))?,
        );
        let mut visited = Vec::new();
        for (key, row) in rows {
            let reference = pb::EntityRef {
                kind: key.kind as i32,
                id: Some(key.id()),
            };
            feed(&reference.encode_to_vec());
            if let Some(row) = row {
                let owner = checked(row)?;
                feed(&owner.encode_to_vec());
                let content =
                    trogon_atlas_core::entity_content_hash(&row.entity).map_err(|_| {
                        Status::failed_precondition("checked entity has no content identity")
                    })?;
                feed(content.to_hex().as_bytes());
                visited.push(owner);
            } else {
                feed(b"absent");
                visited.push(pb::CatalogCheckedOwner {
                    owner: Some(reference),
                    ..Default::default()
                });
            }
        }
        let mut subjects = self.subjects.clone();
        subjects.sort_by_key(prost::Message::encode_to_vec);
        subjects.dedup();
        let mut documents = self.documents.clone();
        documents.sort_by_key(prost::Message::encode_to_vec);
        documents.dedup();
        for subject in &subjects {
            feed(&subject.encode_to_vec());
        }
        for document in &documents {
            feed(&document.encode_to_vec());
        }
        let digest = hash
            .finalize()
            .iter()
            .fold(String::new(), |mut text, byte| {
                const HEX: &[u8; 16] = b"0123456789abcdef";
                text.push(char::from(HEX[usize::from(byte >> 4)]));
                text.push(char::from(HEX[usize::from(byte & 15)]));
                text
            });
        Ok(pb::CatalogCheckedScope {
            branch: self.branch.clone(),
            authorized_namespaces: namespaces.into_iter().collect(),
            revision: format!("catalog-checked-v1:{digest}"),
            visited,
            visited_subjects: subjects,
            visited_documents: documents,
        })
    }
}

impl EventModelServiceImpl {
    async fn catalog_query(
        &self,
        branch: Option<&str>,
        vis: &Visibility,
        scan: bool,
    ) -> Result<CatalogQuery, Status> {
        if let Some(branch) = branch {
            self.require_branch(branch).await?;
        }
        let lens = self.lens(vis).await?;
        let mut rows = if scan {
            self.architecture_reference_snapshot(&[], branch, vis)
                .await?
        } else {
            Vec::new()
        };
        rows.sort_by_key(|row| {
            crate::conv::entity_to_ref(&row.entity)
                .map(|reference| reference.encode_to_vec())
                .unwrap_or_default()
        });
        let mut world = HashMap::new();
        let mut budget = CatalogBudget::default();
        for row in &rows {
            budget.owner(&row.entity)?;
            world.insert(
                pb::EntityKey::new(entity_kind(&row.entity)?, entity_id(&row.entity)?),
                Some(row.clone()),
            );
        }
        Ok(CatalogQuery {
            world,
            rows,
            lens,
            branch: branch.map(str::to_owned),
            budget,
            subjects: Vec::new(),
            documents: Vec::new(),
        })
    }

    async fn catalog_read(
        &self,
        query: &mut CatalogQuery,
        reference: &pb::EntityRef,
    ) -> Result<Option<StoredEntity>, Status> {
        let (kind, id) = exact(reference)?;
        if !query.lens.admits(&id.namespace) {
            return Err(Status::not_found("target not found"));
        }
        let key = pb::EntityKey::new(kind, id);
        if let Some(row) = query.world.get(&key) {
            return Ok(row.clone());
        }
        let row = match self.store.get(kind, id, query.branch.as_deref()).await {
            Ok(row) => {
                query.budget.owner(&row.entity)?;
                Some(row)
            }
            Err(trogon_atlas_store::error::StoreError::NotFound) => None,
            Err(error) => return Err(store_err(error)),
        };
        query.world.insert(key, row.clone());
        Ok(row)
    }

    async fn catalog_subject_state(
        &self,
        query: &mut CatalogQuery,
        subject: &pb::CatalogSubject,
    ) -> Result<State, Status> {
        query.subjects.push(subject.clone());
        if trogon_atlas_core::decision_document::validate_subject(subject).is_err() {
            return Ok(State::Unresolved);
        }
        let converted = resources::responsibility_subject(subject);
        let Some(reference) = crate::responsibility_resources::subject_owner(&converted) else {
            return Ok(State::Unresolved);
        };
        let (_, id) = exact(&reference)?;
        if !query.lens.admits(&id.namespace) {
            return Ok(State::Unavailable);
        }
        let Some(row) = self.catalog_read(query, &reference).await? else {
            return Ok(State::Unavailable);
        };
        query.budget.selection(subject, &row.entity)?;
        if trogon_atlas_core::decision_document::resolve_subject(subject, &row.entity).is_err() {
            return Ok(State::Unresolved);
        }
        for (kind, id) in
            crate::responsibility_resources::selected_endpoints(&converted, &row.entity)
        {
            if !query.lens.admits(&id.namespace) {
                return Ok(State::Unavailable);
            }
            if self
                .catalog_read(
                    query,
                    &pb::EntityRef {
                        kind: kind as i32,
                        id: Some(id),
                    },
                )
                .await?
                .is_none()
            {
                return Ok(State::Unavailable);
            }
        }
        if let Some(selection) =
            crate::responsibility_resources::selected_message(&converted, &row.entity)
        {
            let Some(id) = selection
                .specification
                .as_ref()
                .and_then(|reference| reference.id.as_ref())
            else {
                return Ok(State::Unavailable);
            };
            if !query.lens.admits(&id.namespace) {
                return Ok(State::Unavailable);
            }
            let Some(specification) = self
                .catalog_read(
                    query,
                    &pb::EntityRef {
                        kind: pb::EntityKind::Specification as i32,
                        id: Some(id.clone()),
                    },
                )
                .await?
            else {
                return Ok(State::Unavailable);
            };
            query.budget.selection(
                &pb::CatalogSubject {
                    selection: Some(pb::catalog_subject::Selection::Member(selection.clone())),
                },
                &specification.entity,
            )?;
            if trogon_atlas_core::specification::resolve_member(&specification.entity, &selection)
                .is_err()
            {
                return Ok(State::Unavailable);
            }
        }
        Ok(State::Resolved)
    }

    async fn catalog_document_state(
        &self,
        query: &mut CatalogQuery,
        selection: &pb::DocumentSelection,
    ) -> Result<State, Status> {
        query.documents.push(selection.clone());
        if trogon_atlas_core::decision_document::validate_document_selection(selection).is_err() {
            return Ok(State::Unresolved);
        }
        let reference = trogon_atlas_core::decision_document::document_owner(selection)
            .map_err(|_| Status::invalid_argument("invalid Document selection"))?;
        let (_, id) = exact(&reference)?;
        if !query.lens.admits(&id.namespace) {
            return Ok(State::Unavailable);
        }
        let Some(row) = self.catalog_read(query, &reference).await? else {
            return Ok(State::Unavailable);
        };
        Ok(
            if trogon_atlas_core::decision_document::resolve_document_selection(
                selection,
                &row.entity,
            )
            .is_ok()
            {
                State::Resolved
            } else {
                State::Unresolved
            },
        )
    }

    async fn catalog_decision_state(
        &self,
        query: &mut CatalogQuery,
        selection: &pb::DecisionRef,
        replacement: bool,
    ) -> Result<State, Status> {
        let reference = decision_ref(selection)?;
        let (_, id) = exact(&reference)?;
        if !query.lens.admits(&id.namespace) {
            return Ok(State::Unavailable);
        }
        let Some(row) = self.catalog_read(query, &reference).await? else {
            return Ok(State::Unavailable);
        };
        let Some(pb::entity::Kind::Decision(value)) = &row.entity.kind else {
            return Ok(State::Unresolved);
        };
        Ok(
            if !replacement
                || matches!(
                    pb::DecisionStatus::try_from(value.status),
                    Ok(pb::DecisionStatus::Accepted | pb::DecisionStatus::Superseded)
                )
            {
                State::Resolved
            } else {
                State::Unresolved
            },
        )
    }

    async fn catalog_declarations(
        &self,
        query: &mut CatalogQuery,
        row: &StoredEntity,
    ) -> Result<Vec<pb::CatalogDeclaration>, Status> {
        use pb::catalog_declaration::Declaration as D;
        let owner = checked(row)?;
        let mut declarations = Vec::new();
        for (index, association) in resources::subjects(&row.entity).iter().enumerate() {
            let state = if let Some(subject) = &association.subject {
                self.catalog_subject_state(query, subject).await?
            } else {
                State::Unresolved
            };
            query.budget.output()?;
            declarations.push(pb::CatalogDeclaration {
                owner: Some(owner.clone()),
                field: format!("subjects[{index}].subject"),
                state: state as i32,
                declaration: Some(D::Subject(association.clone())),
            });
        }
        match &row.entity.kind {
            Some(pb::entity::Kind::Decision(value)) => {
                if let Some(replacement) = &value.replaced_by {
                    let state = if let Some(reference) = &replacement.decision {
                        self.catalog_decision_state(query, reference, true).await?
                    } else {
                        State::Unresolved
                    };
                    query.budget.output()?;
                    declarations.push(pb::CatalogDeclaration {
                        owner: Some(owner.clone()),
                        field: "replaced_by.decision".into(),
                        state: state as i32,
                        declaration: Some(D::Replacement(replacement.clone())),
                    });
                }
                for (index, related) in value.related_decisions.iter().enumerate() {
                    let state = if let Some(reference) = &related.decision {
                        self.catalog_decision_state(query, reference, false).await?
                    } else {
                        State::Unresolved
                    };
                    query.budget.output()?;
                    declarations.push(pb::CatalogDeclaration {
                        owner: Some(owner.clone()),
                        field: format!("related_decisions[{index}].decision"),
                        state: state as i32,
                        declaration: Some(D::Related(related.clone())),
                    });
                }
                for (index, supporting) in value.documents.iter().enumerate() {
                    let state = if let Some(selection) = &supporting.document {
                        self.catalog_document_state(query, selection).await?
                    } else {
                        State::Unresolved
                    };
                    query.budget.output()?;
                    declarations.push(pb::CatalogDeclaration {
                        owner: Some(owner.clone()),
                        field: format!("documents[{index}].document"),
                        state: state as i32,
                        declaration: Some(D::Supporting(supporting.clone())),
                    });
                }
            }
            Some(pb::entity::Kind::Document(value)) => {
                for (index, navigation) in value.navigation.iter().enumerate() {
                    let state = if let Some(selection) = &navigation.document {
                        self.catalog_document_state(query, selection).await?
                    } else {
                        State::Unresolved
                    };
                    query.budget.output()?;
                    declarations.push(pb::CatalogDeclaration {
                        owner: Some(owner.clone()),
                        field: format!("navigation[{index}].document"),
                        state: state as i32,
                        declaration: Some(D::Navigation(navigation.clone())),
                    });
                }
            }
            _ => {}
        }
        Ok(declarations)
    }

    async fn catalog_subject_matches(
        &self,
        subject: &pb::CatalogSubject,
        status: Option<i32>,
        decisions: bool,
        branch: Option<&str>,
        vis: &Visibility,
    ) -> Result<(Vec<pb::CatalogDeclaration>, pb::CatalogCheckedScope, State), Status> {
        trogon_atlas_core::decision_document::validate_subject(subject)
            .map_err(|_| Status::invalid_argument("invalid CatalogSubject"))?;
        let reference = trogon_atlas_core::decision_document::subject_owner(subject)
            .map_err(|_| Status::invalid_argument("invalid CatalogSubject"))?;
        let (_, id) = exact(&reference)?;
        self.require_visible(vis, &id.namespace).await?;
        if status.is_some_and(|status| {
            !matches!(
                pb::DecisionStatus::try_from(status),
                Ok(pb::DecisionStatus::Proposed
                    | pb::DecisionStatus::Accepted
                    | pb::DecisionStatus::Rejected
                    | pb::DecisionStatus::Withdrawn
                    | pb::DecisionStatus::Superseded)
            )
        }) {
            return Err(Status::invalid_argument("invalid Decision status filter"));
        }
        let mut query = self.catalog_query(branch, vis, true).await?;
        let state = self.catalog_subject_state(&mut query, subject).await?;
        let mut declarations = Vec::new();
        for row in query.rows.clone() {
            let selected = match &row.entity.kind {
                Some(pb::entity::Kind::Decision(value)) if decisions => {
                    status.is_none_or(|status| value.status == status)
                }
                Some(pb::entity::Kind::Document(_)) => !decisions,
                _ => false,
            };
            if !selected {
                continue;
            }
            for (index, association) in resources::subjects(&row.entity).iter().enumerate() {
                if association.subject.as_ref() != Some(subject) {
                    continue;
                }
                query.budget.output()?;
                declarations.push(pb::CatalogDeclaration {
                    owner: Some(checked(&row)?),
                    field: format!("subjects[{index}].subject"),
                    state: state as i32,
                    declaration: Some(pb::catalog_declaration::Declaration::Subject(
                        association.clone(),
                    )),
                });
            }
        }
        Ok((declarations, query.scope()?, state))
    }

    pub(super) async fn subject_decisions(
        &self,
        req: pb::ListSubjectDecisionsRequest,
        branch: Option<&str>,
        vis: &Visibility,
    ) -> Result<pb::ListSubjectDecisionsResponse, Status> {
        let subject = req
            .subject
            .ok_or_else(|| Status::invalid_argument("subject is required"))?;
        let (declarations, scope, state) = self
            .catalog_subject_matches(&subject, req.status, true, branch, vis)
            .await?;
        Ok(pb::ListSubjectDecisionsResponse {
            declarations,
            scope: Some(scope),
            subject_state: state as i32,
            incomplete: false,
            failures: Vec::new(),
        })
    }

    pub(super) async fn subject_documents(
        &self,
        req: pb::ListSubjectDocumentsRequest,
        branch: Option<&str>,
        vis: &Visibility,
    ) -> Result<pb::ListSubjectDocumentsResponse, Status> {
        let subject = req
            .subject
            .ok_or_else(|| Status::invalid_argument("subject is required"))?;
        let (declarations, scope, state) = self
            .catalog_subject_matches(&subject, None, false, branch, vis)
            .await?;
        Ok(pb::ListSubjectDocumentsResponse {
            declarations,
            scope: Some(scope),
            subject_state: state as i32,
            incomplete: false,
            failures: Vec::new(),
        })
    }

    pub(super) async fn decision_detail(
        &self,
        req: pb::GetDecisionDetailRequest,
        branch: Option<&str>,
        vis: &Visibility,
    ) -> Result<pb::GetDecisionDetailResponse, Status> {
        let reference = decision_ref(
            &req.decision
                .ok_or_else(|| Status::invalid_argument("decision is required"))?,
        )?;
        let mut query = self.catalog_query(branch, vis, false).await?;
        let row = self
            .catalog_read(&mut query, &reference)
            .await?
            .ok_or_else(|| Status::not_found("Decision not found"))?;
        let declarations = self.catalog_declarations(&mut query, &row).await?;
        let mut replacement_chain = Vec::new();
        let mut failures = Vec::new();
        let mut incomplete = false;
        if req.follow_replacement {
            let mut current = row.clone();
            let mut visited = HashSet::new();
            loop {
                query.budget.examine(1)?;
                let key =
                    pb::EntityKey::new(entity_kind(&current.entity)?, entity_id(&current.entity)?);
                if !visited.insert(key) {
                    incomplete = true;
                    break;
                }
                query.budget.output()?;
                replacement_chain.push(checked(&current)?);
                let Some(pb::entity::Kind::Decision(value)) = &current.entity.kind else {
                    incomplete = true;
                    break;
                };
                if value.status == pb::DecisionStatus::Accepted as i32 {
                    break;
                }
                if value.status != pb::DecisionStatus::Superseded as i32 {
                    incomplete = value.replaced_by.is_some() || replacement_chain.len() > 1;
                    break;
                }
                let Some(next) = value
                    .replaced_by
                    .as_ref()
                    .and_then(|replacement| replacement.decision.as_ref())
                else {
                    incomplete = true;
                    break;
                };
                let next = decision_ref(next)?;
                let (_, id) = exact(&next)?;
                if !query.lens.admits(&id.namespace) {
                    incomplete = true;
                    break;
                }
                let Some(next) = self.catalog_read(&mut query, &next).await? else {
                    incomplete = true;
                    break;
                };
                current = next;
            }
            if incomplete {
                failures.push(pb::CatalogQueryFailure {
                    field: "replaced_by".into(),
                    code: tonic::Code::FailedPrecondition as i32,
                    message: "Replacement chain does not establish an available ACCEPTED terminus"
                        .into(),
                    ..Default::default()
                });
            }
        }
        Ok(pb::GetDecisionDetailResponse {
            entity: Some(row.entity),
            etag: row.etag,
            declarations,
            scope: Some(query.scope()?),
            replacement_chain,
            incomplete,
            failures,
        })
    }

    pub(super) async fn document_detail(
        &self,
        req: pb::GetDocumentDetailRequest,
        branch: Option<&str>,
        vis: &Visibility,
    ) -> Result<pb::GetDocumentDetailResponse, Status> {
        let selection = req
            .document
            .ok_or_else(|| Status::invalid_argument("document is required"))?;
        trogon_atlas_core::decision_document::validate_document_selection(&selection)
            .map_err(|_| Status::invalid_argument("invalid Document selection"))?;
        let reference = trogon_atlas_core::decision_document::document_owner(&selection)
            .map_err(|_| Status::invalid_argument("invalid Document selection"))?;
        let mut query = self.catalog_query(branch, vis, false).await?;
        let row = self
            .catalog_read(&mut query, &reference)
            .await?
            .ok_or_else(|| Status::not_found("Document not found"))?;
        let state = self.catalog_document_state(&mut query, &selection).await?;
        let anchor = trogon_atlas_core::decision_document::resolve_document_selection(
            &selection,
            &row.entity,
        )
        .ok()
        .flatten()
        .cloned();
        let declarations = self.catalog_declarations(&mut query, &row).await?;
        Ok(pb::GetDocumentDetailResponse {
            entity: Some(row.entity),
            etag: row.etag,
            anchor,
            declarations,
            scope: Some(query.scope()?),
            selection_state: state as i32,
            incomplete: false,
            failures: Vec::new(),
        })
    }

    pub(super) async fn incoming_decisions(
        &self,
        req: pb::ListIncomingDecisionDeclarationsRequest,
        branch: Option<&str>,
        vis: &Visibility,
    ) -> Result<pb::ListIncomingDecisionDeclarationsResponse, Status> {
        let selection = req
            .decision
            .ok_or_else(|| Status::invalid_argument("decision is required"))?;
        let reference = decision_ref(&selection)?;
        self.require_visible(vis, &exact(&reference)?.1.namespace)
            .await?;
        let mut query = self.catalog_query(branch, vis, true).await?;
        let state = self
            .catalog_decision_state(&mut query, &selection, false)
            .await?;
        let mut declarations = Vec::new();
        for row in query.rows.clone() {
            let Some(pb::entity::Kind::Decision(value)) = &row.entity.kind else {
                continue;
            };
            if let Some(replacement) = value
                .replaced_by
                .as_ref()
                .filter(|replacement| replacement.decision.as_ref() == Some(&selection))
            {
                query.budget.output()?;
                declarations.push(pb::CatalogDeclaration {
                    owner: Some(checked(&row)?),
                    field: "replaced_by.decision".into(),
                    state: self
                        .catalog_decision_state(&mut query, &selection, true)
                        .await? as i32,
                    declaration: Some(pb::catalog_declaration::Declaration::Replacement(
                        replacement.clone(),
                    )),
                });
            }
            for (index, related) in value.related_decisions.iter().enumerate() {
                if related.decision.as_ref() != Some(&selection) {
                    continue;
                }
                query.budget.output()?;
                declarations.push(pb::CatalogDeclaration {
                    owner: Some(checked(&row)?),
                    field: format!("related_decisions[{index}].decision"),
                    state: state as i32,
                    declaration: Some(pb::catalog_declaration::Declaration::Related(
                        related.clone(),
                    )),
                });
            }
        }
        Ok(pb::ListIncomingDecisionDeclarationsResponse {
            declarations,
            scope: Some(query.scope()?),
            incomplete: false,
            failures: Vec::new(),
        })
    }

    pub(super) async fn incoming_documents(
        &self,
        req: pb::ListIncomingDocumentDeclarationsRequest,
        branch: Option<&str>,
        vis: &Visibility,
    ) -> Result<pb::ListIncomingDocumentDeclarationsResponse, Status> {
        let selection = req
            .document
            .ok_or_else(|| Status::invalid_argument("document is required"))?;
        trogon_atlas_core::decision_document::validate_document_selection(&selection)
            .map_err(|_| Status::invalid_argument("invalid Document selection"))?;
        let reference = trogon_atlas_core::decision_document::document_owner(&selection)
            .map_err(|_| Status::invalid_argument("invalid Document selection"))?;
        self.require_visible(vis, &exact(&reference)?.1.namespace)
            .await?;
        let role_filter = req.related_content_role.map(|role| {
            pb::RelatedContentRole::try_from(role)
                .map_err(|_| Status::invalid_argument("invalid related content role filter"))
        });
        let role_filter = match role_filter {
            Some(Ok(value)) => Some(value),
            Some(Err(error)) => return Err(error),
            None => None,
        };
        let page_size = super::clamp_page_size(req.page_size, 100, 500);
        let mut query = self.catalog_query(branch, vis, true).await?;
        let scope = query.scope()?;
        let start = decode_catalog_page_token(&req.page_token, &scope, role_filter)?;
        let state = self.catalog_document_state(&mut query, &selection).await?;
        let mut declarations = Vec::new();
        for row in query.rows.clone() {
            match &row.entity.kind {
                Some(pb::entity::Kind::Decision(value)) => {
                    for (index, supporting) in value.documents.iter().enumerate() {
                        if supporting.document.as_ref() != Some(&selection) {
                            continue;
                        }
                        query.budget.output()?;
                        let field = format!("documents[{index}].document");
                        declarations.push((
                            catalog_declaration_sort_key(
                                &row,
                                &field,
                                supporting.connection_id.as_ref(),
                            )?,
                            pb::CatalogDeclaration {
                                owner: Some(checked(&row)?),
                                field,
                                state: state as i32,
                                declaration: Some(
                                    pb::catalog_declaration::Declaration::Supporting(
                                        supporting.clone(),
                                    ),
                                ),
                            },
                        ));
                    }
                }
                Some(pb::entity::Kind::Document(value)) => {
                    for (index, navigation) in value.navigation.iter().enumerate() {
                        if navigation.document.as_ref() != Some(&selection) {
                            continue;
                        }
                        query.budget.output()?;
                        let field = format!("navigation[{index}].document");
                        declarations.push((
                            catalog_declaration_sort_key(
                                &row,
                                &field,
                                navigation.connection_id.as_ref(),
                            )?,
                            pb::CatalogDeclaration {
                                owner: Some(checked(&row)?),
                                field,
                                state: state as i32,
                                declaration: Some(
                                    pb::catalog_declaration::Declaration::Navigation(
                                        navigation.clone(),
                                    ),
                                ),
                            },
                        ));
                    }
                    for (index, related) in value.related_content.iter().enumerate() {
                        if related.document.as_ref() != Some(&selection) {
                            continue;
                        }
                        let role = pb::RelatedContentRole::try_from(related.role)
                            .map_err(|_| Status::internal("invalid stored related content role"))?;
                        if role_filter.is_some_and(|filter| filter != role) {
                            continue;
                        }
                        query.budget.output()?;
                        let field = format!("related_content[{index}].document");
                        declarations.push((
                            catalog_declaration_sort_key(
                                &row,
                                &field,
                                related.connection_id.as_ref(),
                            )?,
                            pb::CatalogDeclaration {
                                owner: Some(checked(&row)?),
                                field,
                                state: state as i32,
                                declaration: Some(
                                    pb::catalog_declaration::Declaration::RelatedContent(
                                        related.clone(),
                                    ),
                                ),
                            },
                        ));
                    }
                }
                _ => {}
            }
        }
        declarations.sort_by(|left, right| left.0.cmp(&right.0));
        let total = declarations.len();
        let page: Vec<_> = declarations
            .into_iter()
            .skip(start)
            .take(page_size)
            .map(|(_, declaration)| declaration)
            .collect();
        let next = start + page.len();
        let next_page_token = if next < total {
            encode_catalog_page_token(&scope, role_filter, next)?
        } else {
            String::new()
        };
        Ok(pb::ListIncomingDocumentDeclarationsResponse {
            declarations: page,
            scope: Some(scope),
            incomplete: false,
            failures: Vec::new(),
            next_page_token,
        })
    }
}

fn catalog_declaration_sort_key(
    row: &StoredEntity,
    field: &str,
    connection_id: Option<&pb::ConnectionId>,
) -> Result<String, Status> {
    let owner = crate::conv::entity_to_ref(&row.entity)?;
    let id = owner
        .id
        .as_ref()
        .ok_or_else(|| Status::internal("catalog owner missing id"))?;
    let connection = connection_id
        .map(|value| value.value.as_str())
        .unwrap_or_default();
    Ok(format!(
        "{kind:02}:{ns}/{slug}@{version}:{connection}:{field}",
        kind = owner.kind,
        ns = id.namespace,
        slug = id.slug,
        version = id.version,
        connection = connection,
        field = field,
    ))
}

fn encode_catalog_page_token(
    scope: &pb::CatalogCheckedScope,
    role: Option<pb::RelatedContentRole>,
    index: usize,
) -> Result<String, Status> {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
    let payload = serde_json::json!({
        "revision": scope.revision,
        "role": role.map(|value| value as i32),
        "index": index,
    });
    Ok(URL_SAFE_NO_PAD.encode(
        serde_json::to_vec(&payload)
            .map_err(|_| Status::internal("cannot encode catalog page token"))?,
    ))
}

fn decode_catalog_page_token(
    token: &str,
    scope: &pb::CatalogCheckedScope,
    role: Option<pb::RelatedContentRole>,
) -> Result<usize, Status> {
    if token.is_empty() {
        return Ok(0);
    }
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
    let bytes = URL_SAFE_NO_PAD
        .decode(token)
        .map_err(|_| Status::invalid_argument("invalid page token"))?;
    let payload: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|_| Status::invalid_argument("invalid page token"))?;
    let revision = payload
        .get("revision")
        .and_then(|value| value.as_str())
        .ok_or_else(|| Status::invalid_argument("stale page token"))?;
    if revision != scope.revision {
        return Err(Status::invalid_argument("stale page token"));
    }
    let token_role = payload.get("role").and_then(|value| value.as_i64());
    let expected = role.map(|value| i64::from(value as i32));
    if token_role != expected {
        return Err(Status::invalid_argument("page token filter mismatch"));
    }
    usize::try_from(
        payload
            .get("index")
            .and_then(|value| value.as_u64())
            .ok_or_else(|| Status::invalid_argument("invalid page token"))?,
    )
    .map_err(|_| Status::invalid_argument("invalid page token"))
}
