use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};

use pb::ResponsibilityResolutionState as State;
use prost::Message as _;
use sha2::{Digest, Sha256};
use tonic::Status;

use super::{
    entity_id, entity_kind, pb, store_err, EventModelServiceImpl, Lens, StoredEntity, Visibility,
};
use crate::{
    decision_document_resources::occurrence_count as occurrences,
    responsibility_resources::{
        assignments, selected_endpoints, subject_owner, MAX_RESPONSIBILITY_OCCURRENCES,
    },
};

type World = HashMap<pb::EntityKey, Option<StoredEntity>>;

pub(super) struct ResponsibilityQuery {
    world: World,
    rows: Vec<StoredEntity>,
    lens: Lens,
    branch: Option<String>,
    examined: usize,
    examined_owners: HashSet<pb::EntityKey>,
    resource_rows: HashSet<pb::EntityKey>,
    output: usize,
    subjects: Vec<pb::ResponsibilitySubject>,
    claim_budget: crate::decision_document_resources::CatalogBudget,
}

impl ResponsibilityQuery {
    fn examine_within_budget(&mut self, count: usize) -> bool {
        self.examined = self.examined.saturating_add(count);
        self.examined <= MAX_RESPONSIBILITY_OCCURRENCES
    }

    fn examine(&mut self, count: usize) -> Result<(), Status> {
        if !self.examine_within_budget(count) {
            return Err(Status::resource_exhausted(
                "responsibility occurrence budget exceeded",
            ));
        }
        Ok(())
    }

    fn owner(&mut self, entity: &pb::Entity) -> Result<(), Status> {
        let key = pb::EntityKey::new(entity_kind(entity)?, entity_id(entity)?);
        if (crate::validation::architecture_resources::is_resource(entity)
            || matches!(entity.kind, Some(pb::entity::Kind::Schema(_))))
            && self.resource_rows.insert(key.clone())
            && self.resource_rows.len() > 5_000
        {
            return Err(Status::resource_exhausted(
                "responsibility resource budget exceeded",
            ));
        }
        if self.examined_owners.insert(key) {
            self.examine(occurrences(entity))?;
        }
        Ok(())
    }

    fn output(&mut self) -> Result<(), Status> {
        self.output += 1;
        if self.output > 4_096 {
            return Err(Status::resource_exhausted(
                "responsibility output budget exceeded",
            ));
        }
        Ok(())
    }

    fn scope(&self) -> Result<pb::ResponsibilityCheckedScope, Status> {
        let namespaces: BTreeSet<_> = match &self.lens {
            Lens::Everything => self.world.keys().map(|key| key.namespace.clone()).collect(),
            Lens::Only(values) => values
                .iter()
                .map(|value| value.as_str().to_owned())
                .collect(),
        };
        let mut rows: Vec<_> = self.world.iter().collect();
        rows.sort_by_key(|(key, _)| (key.kind as i32, &key.namespace, &key.slug, key.version));
        let mut hash = Sha256::new();
        hash.update(
            serde_json::to_vec(&(
                &self.branch,
                &namespaces,
                matches!(self.lens, Lens::Everything),
            ))
            .map_err(|_| Status::internal("cannot encode responsibility scope"))?,
        );
        let mut visited = Vec::new();
        for (key, row) in rows {
            let reference = pb::EntityRef {
                kind: key.kind as i32,
                id: Some(key.id()),
            };
            hash_field(&mut hash, &reference.encode_to_vec());
            if let Some(row) = row {
                let content = trogon_atlas_core::entity_content_hash(&row.entity)
                    .map_err(|_| {
                        Status::failed_precondition("checked entity has no content identity")
                    })?
                    .to_hex();
                hash_field(&mut hash, content.as_bytes());
                hash_field(&mut hash, uid(&row.entity).as_bytes());
                hash_field(&mut hash, row.etag.as_bytes());
            } else {
                hash_field(&mut hash, b"absent");
            }
            visited.push(reference);
        }
        let mut subjects = self.subjects.clone();
        subjects.sort_by_key(prost::Message::encode_to_vec);
        subjects.dedup();
        for subject in &subjects {
            hash_field(&mut hash, &subject.encode_to_vec());
        }
        Ok(pb::ResponsibilityCheckedScope {
            branch: self.branch.clone(),
            authorized_namespaces: namespaces.into_iter().collect(),
            revision: format!(
                "responsibility-checked-v1:{}",
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
            visited_subjects: subjects,
        })
    }
}

fn hash_field(hash: &mut Sha256, bytes: &[u8]) {
    hash.update((bytes.len() as u64).to_le_bytes());
    hash.update(bytes);
}

fn assignee(entity: &pb::Entity) -> Result<pb::ResponsibilityAssignee, Status> {
    let id = Some(entity_id(entity)?.clone());
    let selection = match entity_kind(entity)? {
        pb::EntityKind::Team => pb::responsibility_assignee::Selection::Team(pb::TeamRef { id }),
        pb::EntityKind::Person => {
            pb::responsibility_assignee::Selection::Person(pb::PersonRef { id })
        }
        _ => return Err(Status::invalid_argument("assignee must be Team or Person")),
    };
    Ok(pb::ResponsibilityAssignee {
        selection: Some(selection),
    })
}

fn assignee_ref(value: &pb::ResponsibilityAssignee) -> Result<pb::EntityRef, Status> {
    let (kind, id) = match &value.selection {
        Some(pb::responsibility_assignee::Selection::Team(value)) => {
            (pb::EntityKind::Team, &value.id)
        }
        Some(pb::responsibility_assignee::Selection::Person(value)) => {
            (pb::EntityKind::Person, &value.id)
        }
        None => return Err(Status::invalid_argument("assignee selection is required")),
    };
    Ok(pb::EntityRef {
        kind: kind as i32,
        id: Some(super::require_id(id.as_ref(), "assignee")?.clone()),
    })
}

fn uid(entity: &pb::Entity) -> String {
    entity
        .system
        .as_ref()
        .map_or_else(String::new, |system| system.uid.clone())
}

fn kind_filter(kind: Option<i32>) -> Result<Option<i32>, Status> {
    if let Some(kind) = kind {
        if !matches!(
            pb::ResponsibilityKind::try_from(kind),
            Ok(pb::ResponsibilityKind::Owner
                | pb::ResponsibilityKind::Maintainer
                | pb::ResponsibilityKind::Steward
                | pb::ResponsibilityKind::Contact)
        ) {
            return Err(Status::invalid_argument("responsibility kind is invalid"));
        }
    }
    Ok(kind)
}

impl EventModelServiceImpl {
    pub(super) async fn responsibility_closure_rows(
        &self,
        root: &pb::EntityRef,
        branch: Option<&str>,
        vis: &Visibility,
    ) -> Result<(Vec<StoredEntity>, bool), Status> {
        let mut query = self.responsibility_query(branch, vis, false).await?;
        let mut pending = VecDeque::from([root.clone()]);
        let mut seen = HashSet::new();
        let mut incomplete = false;
        let mut rows = Vec::new();
        while let Some(reference) = pending.pop_front() {
            let kind = super::kind_from_i32(reference.kind, "closure.kind")?;
            let id = super::require_id(reference.id.as_ref(), "closure.id")?;
            if !query.lens.admits(&id.namespace) {
                incomplete = true;
                continue;
            }
            if !seen.insert(pb::EntityKey::new(kind, id)) {
                continue;
            }
            if seen.len() > 4_096 {
                incomplete = true;
                break;
            }
            let Some(row) = self.responsibility_lookup(&mut query, &reference).await? else {
                incomplete = true;
                continue;
            };
            match query.owner(&row.entity) {
                Ok(()) => {}
                Err(status) if status.code() == tonic::Code::ResourceExhausted => {
                    incomplete = true;
                    break;
                }
                Err(status) => return Err(status),
            }
            for target in trogon_atlas_core::refs::outbound_refs(&row.entity) {
                if let Some(kind) = target.to_kind {
                    pending.push_back(pb::EntityRef {
                        kind: kind as i32,
                        id: Some(target.to_id),
                    });
                } else {
                    let mut resolved = false;
                    for kind in [
                        pb::EntityKind::CommandSlice,
                        pb::EntityKind::ReadModelSlice,
                        pb::EntityKind::AutomationSlice,
                        pb::EntityKind::UiSlice,
                    ] {
                        if !query.lens.admits(&target.to_id.namespace) {
                            incomplete = true;
                            break;
                        }
                        let reference = pb::EntityRef {
                            kind: kind as i32,
                            id: Some(target.to_id.clone()),
                        };
                        if self
                            .responsibility_lookup(&mut query, &reference)
                            .await?
                            .is_some()
                        {
                            pending.push_back(reference);
                            resolved = true;
                            break;
                        }
                    }
                    incomplete |= !resolved;
                }
            }
            rows.push(row);
        }
        Ok((rows, incomplete))
    }

    async fn responsibility_query(
        &self,
        branch: Option<&str>,
        vis: &Visibility,
        scan: bool,
    ) -> Result<ResponsibilityQuery, Status> {
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
        rows.sort_by_key(|row| row.entity.encode_to_vec());
        let mut world = World::new();
        for row in &rows {
            world.insert(
                pb::EntityKey::new(entity_kind(&row.entity)?, entity_id(&row.entity)?),
                Some(row.clone()),
            );
        }
        Ok(ResponsibilityQuery {
            world,
            rows,
            lens,
            branch: branch.map(str::to_owned),
            examined: 0,
            examined_owners: HashSet::new(),
            resource_rows: HashSet::new(),
            output: 0,
            subjects: Vec::new(),
            claim_budget: crate::decision_document_resources::CatalogBudget::default(),
        })
    }

    async fn responsibility_lookup(
        &self,
        query: &mut ResponsibilityQuery,
        reference: &pb::EntityRef,
    ) -> Result<Option<StoredEntity>, Status> {
        let kind = super::kind_from_i32(reference.kind, "subject.kind")?;
        let id = super::require_id(reference.id.as_ref(), "subject.id")?;
        if !query.lens.admits(&id.namespace) {
            return Err(Status::not_found("target not found"));
        }
        let key = pb::EntityKey::new(kind, id);
        if let Some(row) = query.world.get(&key).cloned() {
            return Ok(row);
        }
        let row = match self.store.get(kind, id, query.branch.as_deref()).await {
            Ok(row) => Some(row),
            Err(trogon_atlas_store::error::StoreError::NotFound) => None,
            Err(error) => return Err(store_err(error)),
        };
        query.world.insert(key, row.clone());
        Ok(row)
    }

    async fn responsibility_read(
        &self,
        query: &mut ResponsibilityQuery,
        reference: &pb::EntityRef,
    ) -> Result<Option<StoredEntity>, Status> {
        let row = self.responsibility_lookup(query, reference).await?;
        if let Some(row) = &row {
            query.owner(&row.entity)?;
        }
        Ok(row)
    }

    async fn responsibility_state(
        &self,
        query: &mut ResponsibilityQuery,
        subject: &pb::ResponsibilitySubject,
    ) -> Result<State, Status> {
        query.subjects.push(subject.clone());
        if trogon_atlas_core::responsibility::validate_subject(subject).is_err() {
            return Ok(State::Unresolved);
        }
        let Some(reference) = subject_owner(subject) else {
            return Ok(State::Unresolved);
        };
        let Some(id) = &reference.id else {
            return Ok(State::Unresolved);
        };
        if !query.lens.admits(&id.namespace) {
            return Ok(State::Inaccessible);
        }
        let Some(row) = self.responsibility_read(query, &reference).await? else {
            return Ok(State::Unresolved);
        };
        if trogon_atlas_core::responsibility::resolve_subject(subject, &row.entity).is_err() {
            return Ok(State::Unresolved);
        }
        let architectural = matches!(
            row.entity.kind,
            Some(
                pb::entity::Kind::Component(_)
                    | pb::entity::Kind::ExternalSystem(_)
                    | pb::entity::Kind::Domain(_)
            )
        );
        for (kind, id) in selected_endpoints(subject, &row.entity) {
            if !query.lens.admits(&id.namespace) {
                return Ok(if architectural {
                    State::Unresolved
                } else {
                    State::Inaccessible
                });
            }
            let target = self
                .responsibility_read(
                    query,
                    &pb::EntityRef {
                        kind: kind as i32,
                        id: Some(id),
                    },
                )
                .await?;
            if architectural && target.is_none() {
                return Ok(State::Unresolved);
            }
        }
        if let Some(selected) = crate::message_claims_resources::selected(subject, &row.entity) {
            let targets = match crate::message_claims_resources::native_targets(
                &selected,
                &crate::validation::architecture_resources::ArchitectureContext::new(
                    query.world.values().filter_map(Option::as_ref),
                ),
            ) {
                Ok(targets) => targets,
                Err(_) => return Ok(State::Unresolved),
            };
            for (kind, id) in targets {
                if !query.lens.admits(&id.namespace) {
                    return Ok(State::Unresolved);
                }
                if self
                    .responsibility_read(
                        query,
                        &pb::EntityRef {
                            kind: kind as i32,
                            id: Some(id),
                        },
                    )
                    .await?
                    .is_none()
                {
                    return Ok(State::Unresolved);
                }
            }
            let before = query.claim_budget.examined();
            let valid = {
                let context = crate::validation::architecture_resources::ArchitectureContext::new(
                    query.world.values().filter_map(Option::as_ref),
                );
                crate::message_claims_resources::charge_selections(
                    &selected,
                    &context,
                    &mut query.claim_budget,
                )?;
                crate::message_claims_resources::validate(&selected, &context).is_ok()
            };
            query.examine(query.claim_budget.examined() - before)?;
            return Ok(if valid {
                State::Resolved
            } else {
                State::Unresolved
            });
        }
        if let Some(selection) =
            crate::responsibility_resources::selected_message(subject, &row.entity)
        {
            let Some(id) = selection
                .specification
                .as_ref()
                .and_then(|reference| reference.id.as_ref())
            else {
                return Ok(State::Unresolved);
            };
            if !query.lens.admits(&id.namespace) {
                return Ok(State::Unresolved);
            }
            let Some(specification) = self
                .responsibility_read(
                    query,
                    &pb::EntityRef {
                        kind: pb::EntityKind::Specification as i32,
                        id: Some(id.clone()),
                    },
                )
                .await?
            else {
                return Ok(State::Unresolved);
            };
            if trogon_atlas_core::specification::resolve_member(&specification.entity, &selection)
                .is_err()
            {
                return Ok(State::Unresolved);
            }
        }
        Ok(State::Resolved)
    }

    pub(super) async fn subject_assignments(
        &self,
        req: pb::ListSubjectAssignmentsRequest,
        branch: Option<&str>,
        vis: &Visibility,
    ) -> Result<pb::ListSubjectAssignmentsResponse, Status> {
        let subject = req
            .subject
            .ok_or_else(|| Status::invalid_argument("subject is required"))?;
        trogon_atlas_core::responsibility::validate_subject(&subject)
            .map_err(|_| Status::invalid_argument("invalid responsibility subject"))?;
        let reference = subject_owner(&subject)
            .ok_or_else(|| Status::invalid_argument("subject selection is required"))?;
        let id = super::require_id(reference.id.as_ref(), "subject")?;
        self.require_visible(vis, &id.namespace).await?;
        let filter = kind_filter(req.kind)?;
        let mut query = self.responsibility_query(branch, vis, true).await?;
        let state = self.responsibility_state(&mut query, &subject).await?;
        if state == State::Inaccessible {
            return Err(Status::not_found("target not found"));
        }
        let mut found = Vec::new();
        for row in query.rows.clone() {
            if !matches!(
                row.entity.kind,
                Some(pb::entity::Kind::Team(_) | pb::entity::Kind::Person(_))
            ) {
                continue;
            }
            query.owner(&row.entity)?;
            for assignment in assignments(&row.entity) {
                if assignment.subject.as_ref() == Some(&subject)
                    && filter.is_none_or(|kind| assignment.kind == kind)
                {
                    query.output()?;
                    found.push(pb::ResponsibilityAssignment {
                        assignee: Some(assignee(&row.entity)?),
                        assignee_uid: uid(&row.entity),
                        assignment: Some(assignment.clone()),
                        state: state as i32,
                    });
                }
            }
        }
        Ok(pb::ListSubjectAssignmentsResponse {
            assignments: found,
            scope: Some(query.scope()?),
            incomplete: false,
            subject_state: state as i32,
        })
    }

    pub(super) async fn assignee_responsibilities(
        &self,
        req: pb::ListAssigneeResponsibilitiesRequest,
        branch: Option<&str>,
        vis: &Visibility,
    ) -> Result<pb::ListAssigneeResponsibilitiesResponse, Status> {
        let selection = req
            .assignee
            .ok_or_else(|| Status::invalid_argument("assignee is required"))?;
        let reference = assignee_ref(&selection)?;
        let filter = kind_filter(req.kind)?;
        let mut query = self.responsibility_query(branch, vis, false).await?;
        let row = self
            .responsibility_read(&mut query, &reference)
            .await?
            .ok_or_else(|| Status::not_found("assignee not found"))?;
        query.owner(&row.entity)?;
        let mut found = Vec::new();
        for assignment in assignments(&row.entity) {
            if filter.is_some_and(|kind| assignment.kind != kind) {
                continue;
            }
            query.output()?;
            let state = if let Some(subject) = &assignment.subject {
                self.responsibility_state(&mut query, subject).await?
            } else {
                State::Unresolved
            };
            found.push(pb::ResponsibilityAssignment {
                assignee: Some(selection.clone()),
                assignee_uid: uid(&row.entity),
                assignment: Some(assignment.clone()),
                state: state as i32,
            });
        }
        Ok(pb::ListAssigneeResponsibilitiesResponse {
            assignments: found,
            scope: Some(query.scope()?),
            incomplete: false,
        })
    }

    async fn membership_state(
        &self,
        query: &mut ResponsibilityQuery,
        member: &pb::TeamMember,
    ) -> Result<State, Status> {
        let Some(id) = member.person.as_ref().and_then(|person| person.id.clone()) else {
            return Ok(State::Unresolved);
        };
        if !query.lens.admits(&id.namespace) {
            return Ok(State::Inaccessible);
        }
        Ok(
            if self
                .responsibility_read(
                    query,
                    &pb::EntityRef {
                        kind: pb::EntityKind::Person as i32,
                        id: Some(id),
                    },
                )
                .await?
                .is_some()
            {
                State::Resolved
            } else {
                State::Unresolved
            },
        )
    }

    pub(super) async fn team_members(
        &self,
        team: pb::TeamRef,
        branch: Option<&str>,
        vis: &Visibility,
    ) -> Result<pb::ListTeamMembersResponse, Status> {
        let mut query = self.responsibility_query(branch, vis, false).await?;
        let row = self
            .responsibility_read(
                &mut query,
                &pb::EntityRef {
                    kind: pb::EntityKind::Team as i32,
                    id: team.id.clone(),
                },
            )
            .await?
            .ok_or_else(|| Status::not_found("team not found"))?;
        let Some(pb::entity::Kind::Team(value)) = &row.entity.kind else {
            return Err(Status::internal("invalid stored team"));
        };
        query.owner(&row.entity)?;
        let mut found = Vec::new();
        for member in &value.members {
            query.output()?;
            found.push(pb::ResponsibilityMembership {
                team: Some(team.clone()),
                team_uid: uid(&row.entity),
                membership: Some(member.clone()),
                state: self.membership_state(&mut query, member).await? as i32,
            });
        }
        Ok(pb::ListTeamMembersResponse {
            memberships: found,
            scope: Some(query.scope()?),
            incomplete: false,
        })
    }

    pub(super) async fn person_teams(
        &self,
        person: pb::PersonRef,
        branch: Option<&str>,
        vis: &Visibility,
    ) -> Result<pb::ListPersonTeamsResponse, Status> {
        let mut query = self.responsibility_query(branch, vis, true).await?;
        self.responsibility_read(
            &mut query,
            &pb::EntityRef {
                kind: pb::EntityKind::Person as i32,
                id: person.id.clone(),
            },
        )
        .await?
        .ok_or_else(|| Status::not_found("person not found"))?;
        let mut found = Vec::new();
        for row in query.rows.clone() {
            if let Some(pb::entity::Kind::Team(team)) = &row.entity.kind {
                query.owner(&row.entity)?;
                for member in &team.members {
                    if member.person.as_ref() == Some(&person) {
                        query.output()?;
                        found.push(pb::ResponsibilityMembership {
                            team: Some(pb::TeamRef {
                                id: team.id.clone(),
                            }),
                            team_uid: uid(&row.entity),
                            membership: Some(member.clone()),
                            state: State::Resolved as i32,
                        });
                    }
                }
            }
        }
        Ok(pb::ListPersonTeamsResponse {
            memberships: found,
            scope: Some(query.scope()?),
            incomplete: false,
        })
    }

    pub(super) async fn team_contacts(
        &self,
        req: pb::GetTeamContactsRequest,
        branch: Option<&str>,
        vis: &Visibility,
    ) -> Result<pb::GetTeamContactsResponse, Status> {
        let team = req
            .team
            .ok_or_else(|| Status::invalid_argument("team is required"))?;
        let mut query = self.responsibility_query(branch, vis, false).await?;
        let row = self
            .responsibility_read(
                &mut query,
                &pb::EntityRef {
                    kind: pb::EntityKind::Team as i32,
                    id: team.id.clone(),
                },
            )
            .await?
            .ok_or_else(|| Status::not_found("team not found"))?;
        let Some(pb::entity::Kind::Team(value)) = &row.entity.kind else {
            return Err(Status::internal("invalid stored team"));
        };
        query.examine(value.contacts.len())?;
        let mut team_contacts = Vec::new();
        for contact in &value.contacts {
            query.output()?;
            team_contacts.push(pb::ResponsibilityContact {
                team: Some(team.clone()),
                team_uid: uid(&row.entity),
                contact: Some(contact.clone()),
                state: State::Resolved as i32,
                ..Default::default()
            });
        }
        let mut member_contacts = Vec::new();
        if req.include_members {
            query.examine(value.members.len())?;
            for member in &value.members {
                let state = self.membership_state(&mut query, member).await?;
                let mut contacts = Vec::new();
                if state == State::Resolved {
                    if let Some(id) = member.person.as_ref().and_then(|person| person.id.as_ref()) {
                        if let Some(Some(person)) = query
                            .world
                            .get(&pb::EntityKey::new(pb::EntityKind::Person, id))
                        {
                            if let Some(pb::entity::Kind::Person(value)) = &person.entity.kind {
                                contacts.clone_from(&value.contacts);
                            }
                        }
                    }
                }
                query.examine(contacts.len())?;
                if contacts.is_empty() {
                    query.output()?;
                    member_contacts.push(pb::ResponsibilityContact {
                        team: Some(team.clone()),
                        team_uid: uid(&row.entity),
                        membership_id: member.connection_id.clone(),
                        person: member.person.clone(),
                        state: state as i32,
                        contact: None,
                    });
                } else {
                    for contact in contacts {
                        query.output()?;
                        member_contacts.push(pb::ResponsibilityContact {
                            team: Some(team.clone()),
                            team_uid: uid(&row.entity),
                            membership_id: member.connection_id.clone(),
                            person: member.person.clone(),
                            state: state as i32,
                            contact: Some(contact),
                        });
                    }
                }
            }
        }
        Ok(pb::GetTeamContactsResponse {
            team_contacts,
            member_contacts,
            scope: Some(query.scope()?),
            incomplete: false,
        })
    }
}
