use std::collections::{hash_map::Entry, HashMap, HashSet};

use tonic::Code;
use trogon_atlas_store::store::BranchLandOp;

use super::{
    entity_id, entity_kind, pb, store_err, validation, EventModelServiceImpl, Lens, OpMetaLike,
    Status, StoredEntity, Visibility,
};

struct ArchitectureProposal<'a> {
    overlay: HashMap<pb::EntityKey, Option<&'a pb::Entity>>,
    landing: Option<&'a [BranchLandOp]>,
}

#[derive(Clone, Copy)]
enum ArchitectureTargetAccess<'a> {
    Required(&'a Visibility),
    Diagnostic(&'a Lens),
}

impl EventModelServiceImpl {
    pub(super) async fn validate_architecture_selection(
        &self,
        entity: &pb::Entity,
        base: &pb::Entity,
        branch: &str,
        vis: &Visibility,
    ) -> Result<(), Status> {
        let kind = entity_kind(entity)?;
        let id = entity_id(entity)?.clone();
        let ops = [BranchLandOp::Put {
            kind,
            ours: Box::new(entity.clone()),
            base: Some(Box::new(base.clone())),
            expected_baseline_etag: None,
        }];
        self.validate_architecture_proposal(
            &[OpMetaLike {
                kind,
                id,
                is_put: true,
                entity: Some(entity.clone()),
            }],
            Some(branch),
            vis,
            Some(&ops),
        )
        .await
    }

    pub(super) async fn validate_architecture_writes(
        &self,
        metas: &[OpMetaLike],
        branch: Option<&str>,
        vis: &Visibility,
    ) -> Result<(), Status> {
        self.validate_architecture_proposal(metas, branch, vis, None)
            .await
    }

    pub(super) async fn validate_architecture_landing(
        &self,
        metas: &[OpMetaLike],
        ops: &[BranchLandOp],
        vis: &Visibility,
    ) -> Result<(), Status> {
        self.validate_architecture_proposal(metas, None, vis, Some(ops))
            .await
    }

    async fn validate_architecture_proposal(
        &self,
        metas: &[OpMetaLike],
        branch: Option<&str>,
        vis: &Visibility,
        landing: Option<&[BranchLandOp]>,
    ) -> Result<(), Status> {
        let resources: Vec<_> = metas
            .iter()
            .filter(|meta| {
                meta.is_put
                    && matches!(
                        meta.kind,
                        pb::EntityKind::Channel
                            | pb::EntityKind::DataStore
                            | pb::EntityKind::Specification
                    )
            })
            .filter_map(|meta| meta.entity.as_ref())
            .collect();
        if resources.is_empty() {
            return Ok(());
        }
        let overlay: HashMap<_, _> = metas
            .iter()
            .map(|meta| {
                (
                    pb::EntityKey::new(meta.kind, &meta.id),
                    meta.entity.as_ref().filter(|_| meta.is_put),
                )
            })
            .collect();
        let proposal = ArchitectureProposal { overlay, landing };
        let mut world = HashMap::new();
        let mut required = Vec::new();
        for entity in &resources {
            required.extend(
                trogon_atlas_core::refs::outbound_refs(entity)
                    .into_iter()
                    .filter_map(|reference| reference.to_kind.map(|kind| (kind, reference.to_id))),
            );
        }
        let mut loaded = HashSet::new();
        self.load_architecture_targets(
            &proposal,
            branch,
            ArchitectureTargetAccess::Required(vis),
            required,
            &mut loaded,
            &mut world,
        )
        .await?;
        let endpoints = observation_endpoints(&resources, &world);
        self.load_architecture_targets(
            &proposal,
            branch,
            ArchitectureTargetAccess::Required(vis),
            endpoints,
            &mut loaded,
            &mut world,
        )
        .await?;
        let context: Vec<_> = world.into_values().collect();
        for resource in resources {
            let kind = entity_kind(resource)?;
            let id = entity_id(resource)?;
            let previous = match self.store.get(kind, id, branch).await {
                Ok(stored) => Some(stored.entity),
                Err(trogon_atlas_store::error::StoreError::NotFound) => None,
                Err(error) => return Err(store_err(error)),
            };
            let prepared =
                prepare_proposal_entity(&proposal, kind, id, previous.as_ref(), resource)?;
            let issues = validation::architecture_resources::validate(&prepared, &context);
            if !issues.is_empty() {
                return Err(Status::invalid_argument(super::format_scenario_issues(
                    &issues,
                )));
            }
            for selection in crate::specification_resources::library_types(&prepared) {
                if let (Some(id), Some(message)) = (
                    selection.library.as_ref().and_then(|v| v.id.as_ref()),
                    selection.message.as_ref(),
                ) {
                    self.type_libraries
                        .check_contract_type(id, &message.value, &context)
                        .await?;
                }
            }
        }
        Ok(())
    }

    pub(super) async fn architecture_validation_context(
        &self,
        snapshot: &[StoredEntity],
        branch: Option<&str>,
        vis: &Visibility,
    ) -> Result<Vec<StoredEntity>, Status> {
        let resources: Vec<_> = snapshot
            .iter()
            .map(|stored| &stored.entity)
            .filter(|entity| {
                matches!(
                    entity.kind,
                    Some(
                        pb::entity::Kind::Channel(_)
                            | pb::entity::Kind::DataStore(_)
                            | pb::entity::Kind::Specification(_)
                    )
                )
            })
            .collect();
        if resources.is_empty() {
            return Ok(Vec::new());
        }
        let proposal = ArchitectureProposal {
            overlay: HashMap::new(),
            landing: None,
        };
        let lens = self.lens(vis).await?;
        let access = ArchitectureTargetAccess::Diagnostic(&lens);
        let required = resources
            .iter()
            .flat_map(|entity| {
                trogon_atlas_core::refs::outbound_refs(entity)
                    .into_iter()
                    .filter_map(|reference| reference.to_kind.map(|kind| (kind, reference.to_id)))
            })
            .collect();
        let mut loaded = HashSet::new();
        let mut world = HashMap::new();
        self.load_architecture_targets(
            &proposal,
            branch,
            access,
            required,
            &mut loaded,
            &mut world,
        )
        .await?;
        let endpoints = observation_endpoints(&resources, &world);
        self.load_architecture_targets(
            &proposal,
            branch,
            access,
            endpoints,
            &mut loaded,
            &mut world,
        )
        .await?;
        Ok(world.into_values().collect())
    }

    pub(super) async fn specification_type_diagnostics(
        &self,
        snapshot: &[StoredEntity],
        context: &[StoredEntity],
    ) -> Result<Vec<pb::ValidationIssue>, Status> {
        let mut issues = Vec::new();
        let mut checked = HashMap::new();
        for stored in snapshot {
            for selection in crate::specification_resources::library_types(&stored.entity) {
                let (Some(id), Some(message)) = (
                    selection.library.as_ref().and_then(|v| v.id.as_ref()),
                    selection.message.as_ref(),
                ) else {
                    continue;
                };
                let key = (
                    pb::EntityKey::new(pb::EntityKind::TypeLibrary, id),
                    message.value.clone(),
                );
                let result = match checked.entry(key) {
                    Entry::Occupied(entry) => entry.into_mut(),
                    Entry::Vacant(entry) => entry.insert(
                        self.type_libraries
                            .check_contract_type(id, &message.value, context)
                            .await,
                    ),
                };
                if let Err(status) = result {
                    if !matches!(
                        status.code(),
                        Code::InvalidArgument | Code::FailedPrecondition
                    ) {
                        return Err(status.clone());
                    }
                    issues.push(validation::issue_error(
                        "SPECIFICATION_REFERENCE_UNRESOLVED",
                        "Selected protobuf message is absent, unreadable or does not compile in its exact library".into(),
                        crate::conv::entity_to_ref(&stored.entity).ok(),
                        "messages.schemas.library_type",
                    ));
                }
            }
        }
        Ok(issues)
    }

    pub(super) async fn architecture_reference_snapshot(
        &self,
        snapshot: &[StoredEntity],
        branch: Option<&str>,
        vis: &Visibility,
    ) -> Result<Vec<StoredEntity>, Status> {
        let lens = self.lens(vis).await?;
        let namespaces: Vec<String> = match &lens {
            super::Lens::Everything => Vec::new(),
            super::Lens::Only(namespaces) if namespaces.is_empty() => return Ok(snapshot.to_vec()),
            super::Lens::Only(namespaces) => {
                namespaces.iter().map(|id| id.as_str().to_owned()).collect()
            }
        };
        let resources = self
            .store
            .list(
                super::ListFilter {
                    kinds: &[
                        pb::EntityKind::Channel,
                        pb::EntityKind::DataStore,
                        pb::EntityKind::Specification,
                    ],
                    namespaces: &namespaces,
                    ..Default::default()
                },
                Some(super::MAX_PROJECTION_ENTITIES + 1),
                branch,
            )
            .await
            .map_err(store_err)?;
        if resources.len() > super::MAX_PROJECTION_ENTITIES {
            return Err(Status::resource_exhausted(
                "architecture reference scan exceeds its entity budget",
            ));
        }
        let mut complete: HashMap<_, _> = snapshot
            .iter()
            .filter_map(|stored| {
                Some((
                    pb::EntityKey::new(
                        entity_kind(&stored.entity).ok()?,
                        entity_id(&stored.entity).ok()?,
                    ),
                    stored.clone(),
                ))
            })
            .collect();
        for stored in resources {
            let id = entity_id(&stored.entity)?;
            if lens.admits(&id.namespace) {
                complete.insert(pb::EntityKey::new(entity_kind(&stored.entity)?, id), stored);
            }
        }
        Ok(complete.into_values().collect())
    }

    async fn load_architecture_targets(
        &self,
        proposal: &ArchitectureProposal<'_>,
        branch: Option<&str>,
        access: ArchitectureTargetAccess<'_>,
        targets: Vec<(pb::EntityKind, pb::Id)>,
        loaded: &mut HashSet<pb::EntityKey>,
        world: &mut HashMap<pb::EntityKey, StoredEntity>,
    ) -> Result<(), Status> {
        let mut pending = targets;
        while let Some((kind, id)) = pending.pop() {
            match access {
                ArchitectureTargetAccess::Required(vis) => {
                    self.require_visible(vis, &id.namespace).await?;
                }
                ArchitectureTargetAccess::Diagnostic(lens) => {
                    if !lens.admits(&id.namespace)
                        || id.version == 0
                        || super::require_id(Some(&id), "target").is_err()
                    {
                        continue;
                    }
                }
            }
            let key = pb::EntityKey::new(kind, &id);
            if !loaded.insert(key.clone()) {
                continue;
            }
            let stored = match self.store.get(kind, &id, branch).await {
                Ok(stored) => Some(stored),
                Err(trogon_atlas_store::error::StoreError::NotFound) => None,
                Err(error) => return Err(store_err(error)),
            };
            let next = match proposal.overlay.get(&key) {
                Some(Some(entity)) => Some(StoredEntity {
                    entity: prepare_proposal_entity(
                        proposal,
                        kind,
                        &id,
                        stored.as_ref().map(|stored| &stored.entity),
                        entity,
                    )?,
                    etag: String::new(),
                }),
                Some(None) => None,
                None => stored,
            };
            if let Some(stored) = next {
                if let Some(pb::entity::Kind::TypeLibrary(library)) = &stored.entity.kind {
                    for dependency in &library.dependencies {
                        if let Some(target) = &dependency.id {
                            if target.namespace != id.namespace {
                                return Err(Status::invalid_argument(
                                    "type library imports must remain in their owning namespace",
                                ));
                            }
                            pending.push((pb::EntityKind::TypeLibrary, target.clone()));
                        }
                    }
                }
                world.insert(key, stored);
            }
        }
        Ok(())
    }

    pub(super) async fn validate_architecture_write(
        &self,
        entity: &pb::Entity,
        branch: Option<&str>,
        vis: &Visibility,
    ) -> Result<(), Status> {
        if !matches!(
            entity.kind,
            Some(
                pb::entity::Kind::Channel(_)
                    | pb::entity::Kind::DataStore(_)
                    | pb::entity::Kind::Specification(_)
            )
        ) {
            return Ok(());
        }
        self.validate_architecture_writes(
            &[OpMetaLike {
                kind: entity_kind(entity)?,
                id: entity_id(entity)?.clone(),
                is_put: true,
                entity: Some(entity.clone()),
            }],
            branch,
            vis,
        )
        .await
    }
}

fn prepare_proposal_entity(
    proposal: &ArchitectureProposal<'_>,
    kind: pb::EntityKind,
    id: &pb::Id,
    previous: Option<&pb::Entity>,
    entity: &pb::Entity,
) -> Result<pb::Entity, Status> {
    match proposal.landing.and_then(|ops| {
        ops.iter().find_map(|op| match op {
            BranchLandOp::Put {
                kind: op_kind,
                ours,
                base,
                ..
            } if *op_kind == kind && entity_id(ours).ok() == Some(id) => Some(base.as_deref()),
            _ => None,
        })
    }) {
        Some(base) => {
            trogon_atlas_core::connection_persistence::prepare_landing(base, previous, entity)
        }
        None => trogon_atlas_core::connection_persistence::prepare_entity(previous, entity),
    }
    .map_err(|error| Status::invalid_argument(error.to_string()))
}

fn observation_endpoints(
    resources: &[&pb::Entity],
    world: &HashMap<pb::EntityKey, StoredEntity>,
) -> Vec<(pb::EntityKind, pb::Id)> {
    let mut endpoints = Vec::new();
    for resource in resources {
        let Some(pb::entity::Kind::Channel(channel)) = resource.kind.as_ref() else {
            continue;
        };
        for binding in &channel.bindings {
            let Some(observation) = &binding.observation else {
                continue;
            };
            let Some(owner) = &observation.owner else {
                continue;
            };
            let Some(id) = &owner.id else {
                continue;
            };
            if owner.kind != pb::EntityKind::AutomationSlice as i32 {
                continue;
            }
            let Some(stored) = world.get(&pb::EntityKey::new(pb::EntityKind::AutomationSlice, id))
            else {
                continue;
            };
            let Some(pb::entity::Kind::AutomationSlice(slice)) = stored.entity.kind.as_ref() else {
                continue;
            };
            if let Some(id) = slice
                .source_read_models
                .iter()
                .find(|edge| edge.connection_id.is_some() && edge.connection_id == observation.id)
                .and_then(|edge| edge.read_model.as_ref())
                .and_then(|reference| reference.id.as_ref())
            {
                endpoints.push((pb::EntityKind::ReadModel, id.clone()));
            }
            if let Some(id) = slice
                .processor
                .as_ref()
                .and_then(|edge| edge.processor.as_ref())
                .and_then(|reference| reference.id.as_ref())
            {
                endpoints.push((pb::EntityKind::Processor, id.clone()));
            }
        }
    }
    endpoints
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod specification_validation_tests {
    use std::{sync::Arc, time::Duration};

    use tonic::{Code, Request};
    use tonic_types::StatusExt;
    use trogon_atlas_proto::event_model_service_server::EventModelService;
    use trogon_atlas_store::WriteContext;
    use trogon_atlas_types::CompileRunner;

    use super::*;
    use crate::{metrics::ServerMetrics, type_libraries::TypeLibraryGate};

    fn library_id(namespace: &str, version: u64) -> pb::Id {
        pb::Id {
            namespace: namespace.into(),
            slug: "orders".into(),
            version,
        }
    }

    async fn fixture() -> EventModelServiceImpl {
        let nats = trogon_atlas_testsupport::shared().await;
        EventModelServiceImpl::try_new(Arc::new(nats.store().await), ServerMetrics::default())
            .unwrap()
    }

    async fn library(
        service: &EventModelServiceImpl,
        id: pb::Id,
        source: &str,
        dependencies: Vec<pb::TypeLibraryRef>,
    ) {
        service
            .store
            .put(
                pb::EntityKind::TypeLibrary,
                &pb::Entity {
                    kind: Some(pb::entity::Kind::TypeLibrary(pb::TypeLibrary {
                        id: Some(id),
                        title: "Orders".into(),
                        files: vec![pb::ProtoSourceFile {
                            path: "orders.proto".into(),
                            content: source.into(),
                        }],
                        dependencies,
                        ..Default::default()
                    })),
                    ..Default::default()
                },
                false,
                WriteContext::baseline(),
            )
            .await
            .unwrap();
    }

    const VALID: &str = "syntax = \"proto3\"; package orders.v1; message Order {}";

    async fn selection(service: &EventModelServiceImpl, slug: &str, id: pb::Id, message: &str) {
        service
            .put_entity(Request::new(pb::PutEntityRequest {
                entity: Some(pb::Entity {
                    kind: Some(pb::entity::Kind::Specification(pb::Specification {
                        id: Some(pb::Id {
                            namespace: "contracts".into(),
                            slug: slug.into(),
                            version: 1,
                        }),
                        title: slug.into(),
                        messages: vec![pb::ContractMessage {
                            name: Some(pb::MemberName {
                                value: "Payload".into(),
                            }),
                            schemas: vec![pb::SchemaBinding {
                                role: pb::schema_binding::Role::Payload as i32,
                                r#type: Some(pb::schema_binding::Type::LibraryType(
                                    pb::TypeLibraryTypeRef {
                                        library: Some(pb::TypeLibraryRef { id: Some(id) }),
                                        message: Some(pb::QualifiedProtoName {
                                            value: message.into(),
                                        }),
                                    },
                                )),
                                ..Default::default()
                            }],
                            ..Default::default()
                        }],
                        ..Default::default()
                    })),
                    ..Default::default()
                }),
                ..Default::default()
            }))
            .await
            .unwrap();
    }

    async fn validate(
        service: &EventModelServiceImpl,
    ) -> Result<tonic::Response<pb::ValidateProjectResponse>, Status> {
        service
            .validate_project(Request::new(pb::ValidateProjectRequest {
                scope: Some(pb::validate_project_request::Scope::Namespace(
                    "contracts".into(),
                )),
            }))
            .await
    }

    fn issues(report: &pb::ValidateProjectResponse) -> Vec<&pb::ValidationIssue> {
        report
            .reports
            .iter()
            .flat_map(|r| &r.issues)
            .filter(|i| {
                i.code == "SPECIFICATION_REFERENCE_UNRESOLVED"
                    && i.subject_field == "messages.schemas.library_type"
            })
            .collect()
    }

    #[tokio::test]
    async fn validate_project_propagates_compile_timeout() {
        let mut service = fixture().await;
        library(&service, library_id("types", 1), VALID, vec![]).await;
        selection(&service, "typed", library_id("types", 1), "orders.v1.Order").await;
        service.type_libraries =
            TypeLibraryGate::with_compile_runner(CompileRunner::new(0, Duration::from_millis(1)));
        let error = validate(&service)
            .await
            .expect_err("compiler timeout must fail the RPC, not diagnose the model");
        assert_eq!(error.code(), Code::DeadlineExceeded);
        assert_eq!(
            error.get_error_details().error_info().unwrap().reason,
            crate::conv::reason::UNAVAILABLE
        );
        for private in ["types", "orders", "Order", "orders.proto"] {
            assert!(!format!("{error:?}").contains(private));
        }
    }

    #[tokio::test]
    async fn validate_project_memoizes_exact_selections_only_within_each_call() {
        let service = fixture().await;
        for (namespace, version) in [("types", 1), ("types", 2), ("other-types", 1)] {
            library(&service, library_id(namespace, version), VALID, vec![]).await;
        }
        for (slug, namespace, version) in [
            ("first", "types", 1),
            ("duplicate", "types", 1),
            ("version", "types", 2),
            ("namespace", "other-types", 1),
        ] {
            selection(
                &service,
                slug,
                library_id(namespace, version),
                "orders.v1.Order",
            )
            .await;
        }
        let before = service.type_libraries.contract_compile_calls();
        let report = validate(&service).await.unwrap().into_inner();
        assert!(issues(&report).is_empty(), "{report:?}");
        assert_eq!(service.type_libraries.contract_compile_calls() - before, 3);
        library(
            &service,
            library_id("types", 1),
            "syntax = \"proto3\"; package orders.v1; message Replacement {}",
            vec![],
        )
        .await;
        let report = validate(&service).await.unwrap().into_inner();
        let failures = issues(&report);
        assert_eq!(failures.len(), 2, "{report:?}");
        for slug in ["first", "duplicate"] {
            assert!(
                failures.iter().any(|i| i
                    .subject
                    .as_ref()
                    .and_then(|r| r.id.as_ref())
                    .is_some_and(|id| id.slug == slug)),
                "{report:?}"
            );
        }
        assert_eq!(service.type_libraries.contract_compile_calls() - before, 6);
    }

    #[tokio::test]
    async fn validate_project_distinguishes_message_selections_and_redacts_model_errors() {
        let service = fixture().await;
        library(
            &service,
            library_id("types", 1),
            "syntax = \"proto3\"; package orders.v1; message Order {} message Replacement {}",
            vec![],
        )
        .await;
        selection(
            &service,
            "valid",
            library_id("types", 1),
            "orders.v1.Replacement",
        )
        .await;
        selection(
            &service,
            "removed",
            library_id("types", 1),
            "orders.v1.Order",
        )
        .await;
        library(
            &service,
            library_id("types", 1),
            "syntax = \"proto3\"; package orders.v1; message Replacement {}",
            vec![],
        )
        .await;
        let report = validate(&service).await.unwrap().into_inner();
        assert_eq!(issues(&report).len(), 1, "{report:?}");
        assert_eq!(
            issues(&report)[0]
                .subject
                .as_ref()
                .unwrap()
                .id
                .as_ref()
                .unwrap()
                .slug,
            "removed"
        );
        for (source, dependencies) in [
            ("syntax = \"proto3\"; private compiler source", vec![]),
            (
                VALID,
                vec![pb::TypeLibraryRef {
                    id: Some(pb::Id {
                        namespace: "types".into(),
                        slug: "secret-dependency".into(),
                        version: 1,
                    }),
                }],
            ),
        ] {
            library(&service, library_id("types", 1), source, dependencies).await;
            let report = validate(&service).await.unwrap().into_inner();
            assert_eq!(issues(&report).len(), 2, "{report:?}");
            for issue in issues(&report) {
                for private in [
                    "orders.proto",
                    "secret-dependency",
                    "private compiler source",
                    "orders.v1",
                ] {
                    assert!(!format!("{issue:?}").contains(private), "{issue:?}");
                }
            }
        }
    }
}
