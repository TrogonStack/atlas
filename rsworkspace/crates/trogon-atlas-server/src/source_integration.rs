use std::{
    collections::BTreeSet,
    fmt::Write as _,
    sync::{Arc, Mutex},
};

use prost::Message;
use sha2::{Digest, Sha256};
use tonic::{Request, Response, Status};
use trogon_atlas_core::{source_bundle as bundle, source_integration as core};
use trogon_atlas_proto as pb;
use trogon_atlas_store::{
    source::{SourceDocumentKey, SourcePublication, SourcePublicationOutcome},
    StoreError,
};

use super::{entity_id, entity_kind, store_err, EventModelServiceImpl, Visibility, WriteTargets};

#[derive(Clone, PartialEq, Message)]
struct SourceState {
    #[prost(message, optional, tag = "1")]
    registration: Option<pb::SourceRegistration>,
    #[prost(message, repeated, tag = "2")]
    snapshots: Vec<pb::SourceSnapshot>,
    #[prost(message, repeated, tag = "3")]
    mappings: Vec<pb::SourceMapping>,
    #[prost(message, repeated, tag = "4")]
    runs: Vec<StoredRun>,
    #[prost(message, repeated, tag = "5")]
    tombstones: Vec<pb::SourceIdentityTombstone>,
    #[prost(bool, tag = "6")]
    deleted: bool,
    #[prost(message, repeated, tag = "7")]
    overlays: Vec<StoredOverlay>,
}
#[derive(Clone, PartialEq, Message)]
struct StoredOverlay {
    #[prost(string, tag = "1")]
    snapshot_id: String,
    #[prost(message, repeated, tag = "2")]
    entries: Vec<pb::SourceOverlayEntry>,
}
#[derive(Clone, PartialEq, Message)]
struct StoredRun {
    #[prost(message, optional, tag = "1")]
    run: Option<pb::ReconciliationRun>,
    #[prost(message, repeated, tag = "2")]
    payloads: Vec<pb::SourceNativePayload>,
    #[prost(message, repeated, tag = "3")]
    values: Vec<pb::SourceCommittedValue>,
    #[prost(uint32, tag = "4")]
    transitions: u32,
}
struct Document {
    key: SourceDocumentKey,
    revision: u64,
    state: SourceState,
}
struct Context {
    principal: Option<crate::auth::Principal>,
    vis: Visibility,
    branch: Option<String>,
    metadata: tonic::metadata::MetadataMap,
}
/// Marks a source preview or apply batch. `preview_source_run_attempt` inserts
/// it before the validate-only `batch_mutate`. `apply_source_run_attempt`
/// inserts `SourceExecution` and then this flag before the real `batch_mutate`.
/// The flag is not a blanket admission skip: only a stored schema whose
/// aggregate root, identifiers, and property relationships are unchanged may
/// skip target admission, so a title update can keep a dangling target property.
#[derive(Clone, Copy)]
pub(super) struct SourceBusinessBypass;

#[derive(Clone)]
pub(super) struct SourceExecution {
    pub(super) publication: SourcePublication,
    pub(super) outcome: Arc<Mutex<Option<SourcePublicationOutcome>>>,
}
fn now() -> i64 {
    chrono::Utc::now().timestamp_millis()
}
fn revision() -> String {
    uuid::Uuid::now_v7().to_string()
}
fn digest(message: &impl Message) -> String {
    digest_bytes(&message.encode_to_vec())
}
fn digest_bytes(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .fold(String::with_capacity(64), |mut output, byte| {
            let _ = write!(output, "{byte:02x}");
            output
        })
}
fn invalid() -> Status {
    Status::invalid_argument("invalid source control request")
}
fn conflict() -> Status {
    Status::failed_precondition("source state changed; create a fresh guarded preview")
}
fn missing() -> Status {
    Status::not_found("source record unavailable")
}
fn source_error(error: core::SourceError) -> Status {
    match error {
        core::SourceError::Budget => Status::resource_exhausted("source work budget exhausted"),
        core::SourceError::NativeSchemaVariants => Status::invalid_argument(error.to_string()),
        _ => invalid(),
    }
}
fn bounded<T>(value: Result<T, core::SourceError>) -> Result<T, Status> {
    value.map_err(source_error)
}

fn token(value: &str) -> Result<(), Status> {
    bundle::RegistrationId::new(value)
        .map(|_| ())
        .map_err(|_| invalid())
}
fn guard(actual: &str, expected: &str) -> Result<(), Status> {
    if expected.is_empty() || actual != expected {
        Err(conflict())
    } else {
        Ok(())
    }
}
fn registration(state: &SourceState) -> Result<&pb::SourceRegistration, Status> {
    if state.deleted {
        return Err(missing());
    }
    state.registration.as_ref().ok_or_else(missing)
}
fn config_guard(doc: &Document, expected: &str) -> Result<(), Status> {
    guard(&registration(&doc.state)?.revision, expected)
}
fn ensure_settled(state: &SourceState) -> Result<(), Status> {
    if state.runs.iter().any(|r| {
        r.run.as_ref().is_some_and(|r| {
            r.recovery_required
                || r.status == pb::SourceRunStatus::Applying as i32
                || r.status == pb::SourceRunStatus::OutcomeUnknown as i32
        })
    }) {
        return Err(Status::failed_precondition(
            "source recovery is required before changing control state",
        ));
    }
    Ok(())
}
fn get_run<'a>(state: &'a SourceState, id: &str) -> Result<&'a StoredRun, Status> {
    state
        .runs
        .iter()
        .find(|r| r.run.as_ref().is_some_and(|r| r.id == id))
        .ok_or_else(missing)
}
fn run_index(state: &SourceState, id: &str) -> Result<usize, Status> {
    state
        .runs
        .iter()
        .position(|r| r.run.as_ref().is_some_and(|r| r.id == id))
        .ok_or_else(missing)
}
fn summary(run: &pb::ReconciliationRun) -> pb::ReconciliationRun {
    let mut value = run.clone();
    value.result_count = value
        .result_count
        .max(u32::try_from(value.results.len()).unwrap_or(u32::MAX));
    value.operation_count = value.operation_count.max(
        value
            .plan
            .as_ref()
            .map_or(0, |p| u32::try_from(p.operations.len()).unwrap_or(u32::MAX)),
    );
    value.details_omitted = true;
    value.plan = None;
    value.marker = None;
    value.results.clear();
    value.fidelity = None;
    value
}
fn snapshot_summary(snapshot: &pb::SourceSnapshot) -> pb::SourceSnapshot {
    let mut value = snapshot.clone();
    value.inventory_bytes.clear();
    value.files.clear();
    value
}
fn snapshot_deadline(snapshot: &pb::SourceSnapshot, seconds: u64) -> i64 {
    snapshot.expires_at_unix_ms.min(
        snapshot
            .accepted_at_unix_ms
            .unwrap_or(snapshot.created_at_unix_ms)
            .saturating_add(seconds.cast_signed() * 1000),
    )
}
fn expire(state: &mut SourceState) {
    let now = now();
    let retention = state
        .registration
        .as_ref()
        .and_then(|r| r.retention.as_ref())
        .copied()
        .unwrap_or_else(core::profile_retention);
    let protected: BTreeSet<String> = state
        .runs
        .iter()
        .filter_map(|r| r.run.as_ref())
        .filter(|r| r.recovery_required)
        .map(|r| r.snapshot_id.clone())
        .collect();
    for snapshot in &mut state.snapshots {
        if !protected.contains(&snapshot.id)
            && snapshot_deadline(snapshot, retention.raw_bytes_seconds) <= now
        {
            snapshot.inventory_bytes.clear();
            snapshot.files.clear();
            snapshot.bytes_available = false;
        }
    }
    let expired_overlays: BTreeSet<_> = state
        .snapshots
        .iter()
        .filter(|snapshot| {
            !protected.contains(&snapshot.id)
                && snapshot_deadline(snapshot, retention.overlay_seconds) <= now
        })
        .map(|snapshot| snapshot.id.clone())
        .collect();
    state
        .overlays
        .retain(|overlay| !expired_overlays.contains(&overlay.snapshot_id));
    for stored in &mut state.runs {
        if let Some(run) = &mut stored.run {
            if let Some(report) = &mut run.fidelity {
                if state
                    .snapshots
                    .iter()
                    .find(|snapshot| snapshot.id == run.snapshot_id)
                    .is_none_or(|snapshot| !snapshot.bytes_available)
                {
                    report.original_bytes_recoverable = false;
                    for record in &mut report.records {
                        record.original_bytes_recoverable = false;
                    }
                }
                if expired_overlays.contains(&run.snapshot_id) {
                    for record in &mut report.records {
                        if record.disposition
                            == pb::SourceFidelityDisposition::RetainedOverlayOnly as i32
                        {
                            record.disposition = pb::SourceFidelityDisposition::Deferred as i32;
                            record.reason_code = "source_overlay_expired".into();
                        }
                    }
                }
            }

            if !run.recovery_required
                && run.settled_at_unix_ms.is_some_and(|at| {
                    at.saturating_add(retention.marker_seconds.cast_signed() * 1000) <= now
                })
            {
                run.marker = None;
            }
            if !run.recovery_required
                && run.settled_at_unix_ms.is_some_and(|at| {
                    at.saturating_add(retention.audit_seconds.cast_signed() * 1000) <= now
                })
            {
                stored.payloads.clear();
                stored.values.clear();
            }
            if run.expires_at_unix_ms <= now && !run.recovery_required {
                if run.status == pb::SourceRunStatus::Previewed as i32
                    || run.status == pb::SourceRunStatus::Prepared as i32
                {
                    run.status = pb::SourceRunStatus::Rejected as i32;
                }
                if let Some(plan) = &mut run.plan {
                    plan.operations.clear();
                    plan.mapping_guards.clear();
                    for guard in &mut plan.native_guards {
                        guard.condition = None;
                    }
                }
                if run.settled_at_unix_ms.is_none() {
                    run.marker = None;
                    stored.payloads.clear();
                    stored.values.clear();
                }
                run.results.clear();
                run.fidelity = None;
                run.details_omitted = true;
            }
        }
    }
    let protected_mappings: BTreeSet<_> = state
        .runs
        .iter()
        .filter_map(|stored| stored.run.as_ref())
        .filter(|run| run.recovery_required)
        .filter_map(|run| run.plan.as_ref())
        .flat_map(|plan| {
            plan.mapping_guards
                .iter()
                .map(|guard| guard.mapping_id.clone())
        })
        .collect();
    for mapping in &mut state.mappings {
        if let Some(base) = &mut mapping.base {
            if base.expires_at_unix_ms <= now && !protected_mappings.contains(&mapping.id) {
                base.source_value = None;
                base.native_value = None;
            }
        }
    }
}
impl Context {
    fn new<T>(request: &Request<T>, role: crate::auth::Role) -> Result<Self, Status> {
        let principal = EventModelServiceImpl::principal_from_metadata(request);
        if principal.as_ref().is_some_and(|p| p.role < role) {
            return Err(Status::permission_denied("source capability denied"));
        }
        let branch = EventModelServiceImpl::branch_from_metadata(request)?;
        if branch
            .as_deref()
            .is_some_and(|b| !EventModelServiceImpl::branch_visible(principal.as_ref(), b))
        {
            return Err(missing());
        }
        Ok(Self {
            principal,
            vis: EventModelServiceImpl::visibility_of(request),
            branch,
            metadata: request.metadata().clone(),
        })
    }
    async fn authorize(
        &self,
        service: &EventModelServiceImpl,
        registration: &pb::SourceRegistration,
        write: bool,
    ) -> Result<(), Status> {
        service
            .require_visible(&self.vis, &registration.administration_namespace)
            .await?;
        if write {
            let mut targets = WriteTargets::none();
            targets.insert(&registration.administration_namespace);
            service
                .require_namespace_scope(
                    self.principal.as_ref(),
                    &targets,
                    &trogon_atlas_store::NamespaceTenure::Permanent,
                )
                .await?;
        }
        Ok(())
    }
    fn request<T>(&self, value: T) -> Request<T> {
        let mut request = Request::new(value);
        *request.metadata_mut() = self.metadata.clone();
        if let Some(principal) = &self.principal {
            request.extensions_mut().insert(principal.clone());
        }
        request
    }
}
async fn load(
    service: &EventModelServiceImpl,
    ctx: &Context,
    id: &str,
    write: bool,
) -> Result<Document, Status> {
    let key = SourceDocumentKey::new(id).map_err(store_err)?;
    let document = service
        .store
        .source_get(&key)
        .await
        .map_err(store_err)?
        .ok_or_else(missing)?;
    let mut state = SourceState::decode(document.bytes.as_slice())
        .map_err(|_| Status::unavailable("source state cannot be reconstructed"))?;
    ctx.authorize(service, registration(&state)?, write).await?;
    expire(&mut state);
    Ok(Document {
        key,
        revision: document.revision,
        state,
    })
}
async fn save(service: &EventModelServiceImpl, doc: &mut Document) -> Result<(), Status> {
    expire(&mut doc.state);
    let limit = registration(&doc.state)
        .or_else(|_| doc.state.registration.as_ref().ok_or_else(missing))?
        .retention
        .as_ref()
        .ok_or_else(invalid)?
        .registration_bytes;
    let bytes = doc.state.encode_to_vec();
    if bytes.len() as u64 > limit {
        return Err(Status::resource_exhausted(
            "source registration storage budget exhausted",
        ));
    }
    doc.revision = service
        .store
        .source_compare_and_swap(&doc.key, Some(doc.revision), &bytes, limit)
        .await
        .map_err(store_err)?
        .revision;
    Ok(())
}
async fn existing_namespace(
    service: &EventModelServiceImpl,
    ctx: &Context,
    namespace: &str,
) -> Result<(), Status> {
    let id = trogon_atlas_core::NamespaceId::parse(namespace).map_err(|_| invalid())?;
    if service
        .store
        .get_namespace(&id)
        .await
        .map_err(store_err)?
        .is_none()
    {
        return Err(missing());
    }
    service.require_visible(&ctx.vis, namespace).await
}
async fn validate_registration(
    service: &EventModelServiceImpl,
    ctx: &Context,
    value: &mut pb::SourceRegistration,
) -> Result<(), Status> {
    token(&value.id)?;
    token(&value.ownership_policy_revision)?;
    if value.profile_revision != 1
        || value.destination_namespaces.is_empty()
        || value.destination_namespaces.len() > 1000
        || pb::SourceDeletionPolicy::try_from(value.deletion_policy).is_err()
        || value.deletion_policy == 0
    {
        return Err(invalid());
    }
    existing_namespace(service, ctx, &value.administration_namespace).await?;
    let mut seen = BTreeSet::new();
    for namespace in &value.destination_namespaces {
        if !seen.insert(namespace) {
            return Err(invalid());
        }
        existing_namespace(service, ctx, namespace).await?;
    }
    let mut targets = WriteTargets::none();
    for namespace in &value.destination_namespaces {
        targets.insert(namespace);
    }
    targets.insert(&value.administration_namespace);
    service
        .require_namespace_scope(
            ctx.principal.as_ref(),
            &targets,
            &trogon_atlas_store::NamespaceTenure::Permanent,
        )
        .await?;
    bounded(core::validate_budgets(
        value.budgets.get_or_insert_with(core::profile_budgets),
    ))?;
    bounded(core::validate_retention(
        value.retention.get_or_insert_with(core::profile_retention),
    ))?;
    Ok(())
}
pub(super) async fn create_source_registration(
    service: &EventModelServiceImpl,
    request: Request<pb::CreateSourceRegistrationRequest>,
) -> Result<Response<pb::CreateSourceRegistrationResponse>, Status> {
    let ctx = Context::new(&request, crate::auth::Role::Admin)?;
    let mut value = request.into_inner().registration.ok_or_else(invalid)?;
    if !value.revision.is_empty() || value.created_at_unix_ms != 0 {
        return Err(invalid());
    }
    let _lock = service.lock_mutations().await;
    validate_registration(service, &ctx, &mut value).await?;
    let key = SourceDocumentKey::new(&value.id).map_err(store_err)?;
    let previous = service.store.source_get(&key).await.map_err(store_err)?;
    if let Some(previous) = &previous {
        let state = SourceState::decode(previous.bytes.as_slice()).map_err(|_| conflict())?;
        if !state.deleted {
            return Err(Status::already_exists("source registration exists"));
        }
    }
    value.revision = revision();
    value.created_at_unix_ms = now();
    let state = SourceState {
        registration: Some(value.clone()),
        ..Default::default()
    };
    service
        .store
        .source_compare_and_swap(
            &key,
            previous.map(|d| d.revision),
            &state.encode_to_vec(),
            value
                .retention
                .as_ref()
                .ok_or_else(invalid)?
                .registration_bytes,
        )
        .await
        .map_err(store_err)?;
    Ok(Response::new(pb::CreateSourceRegistrationResponse {
        registration: Some(value),
    }))
}
pub(super) async fn update_source_registration(
    service: &EventModelServiceImpl,
    request: Request<pb::UpdateSourceRegistrationRequest>,
) -> Result<Response<pb::UpdateSourceRegistrationResponse>, Status> {
    let ctx = Context::new(&request, crate::auth::Role::Admin)?;
    let request = request.into_inner();
    let mut value = request.registration.ok_or_else(invalid)?;
    let _lock = service.lock_mutations().await;
    let mut doc = load(service, &ctx, &value.id, true).await?;
    ensure_settled(&doc.state)?;
    config_guard(&doc, &request.if_match)?;
    let old = registration(&doc.state)?;
    if old.administration_namespace != value.administration_namespace
        || value.created_at_unix_ms != old.created_at_unix_ms
    {
        return Err(invalid());
    }
    validate_registration(service, &ctx, &mut value).await?;
    value.revision = revision();
    doc.state.registration = Some(value.clone());
    save(service, &mut doc).await?;
    Ok(Response::new(pb::UpdateSourceRegistrationResponse {
        registration: Some(value),
    }))
}
pub(super) async fn get_source_registration(
    service: &EventModelServiceImpl,
    request: Request<pb::GetSourceRegistrationRequest>,
) -> Result<Response<pb::GetSourceRegistrationResponse>, Status> {
    let ctx = Context::new(&request, crate::auth::Role::Reader)?;
    let doc = load(service, &ctx, &request.into_inner().registration_id, false).await?;
    Ok(Response::new(pb::GetSourceRegistrationResponse {
        registration: doc.state.registration,
    }))
}
fn page_start(token: &str) -> Result<usize, Status> {
    if token.is_empty() {
        Ok(0)
    } else {
        token.parse::<usize>().map_err(|_| invalid())
    }
}
fn page_limit(size: u32, maximum: u32) -> Result<usize, Status> {
    if size > maximum {
        return Err(invalid());
    }
    Ok(if size == 0 { maximum.min(100) } else { size } as usize)
}
fn page<T: Clone + Message>(
    items: &[T],
    start: usize,
    limit: usize,
    max_bytes: u64,
) -> Result<(Vec<T>, String), Status> {
    if start > items.len() {
        return Err(invalid());
    }
    let mut bytes = 0u64;
    let mut end = start;
    while end < items.len() && end - start < limit {
        let size = items[end].encoded_len() as u64 + 16;
        if bytes.saturating_add(size) > max_bytes {
            if end == start {
                return Err(Status::resource_exhausted(
                    "source record exceeds page byte budget",
                ));
            }
            break;
        }
        bytes += size;
        end += 1;
    }
    Ok((
        items[start..end].to_vec(),
        if end < items.len() {
            end.to_string()
        } else {
            String::new()
        },
    ))
}
pub(super) async fn list_source_registrations(
    service: &EventModelServiceImpl,
    request: Request<pb::ListSourceRegistrationsRequest>,
) -> Result<Response<pb::ListSourceRegistrationsResponse>, Status> {
    let ctx = Context::new(&request, crate::auth::Role::Reader)?;
    let req = request.into_inner();
    if !req.administration_namespace.is_empty() {
        service
            .require_visible(&ctx.vis, &req.administration_namespace)
            .await?;
    }
    let limit = page_limit(req.page_size, 500)?;
    let start = page_start(&req.page_token)?;
    let mut registrations = Vec::new();
    let mut cursor = None;
    let mut visible = 0usize;
    loop {
        let roots = service
            .store
            .source_list(cursor.as_deref(), 500)
            .await
            .map_err(store_err)?;
        for key in roots.keys {
            match load(service, &ctx, key.as_str(), false).await {
                Ok(doc) => {
                    let value = registration(&doc.state)?;
                    if !req.administration_namespace.is_empty()
                        && req.administration_namespace != value.administration_namespace
                    {
                        continue;
                    }
                    if visible >= start {
                        registrations.push(value.clone());
                    }
                    visible += 1;
                    if registrations.len() > limit {
                        break;
                    }
                }
                Err(status) if status.code() == tonic::Code::NotFound => {}
                Err(status) => return Err(status),
            }
        }
        cursor = roots.next_cursor;
        if registrations.len() > limit || cursor.is_none() {
            break;
        }
    }
    if start > visible {
        return Err(invalid());
    }
    let (registrations, continuation) = page(&registrations, 0, limit, 4 * 1024 * 1024)?;
    let next_page_token = if continuation.is_empty() {
        String::new()
    } else {
        (start + registrations.len()).to_string()
    };
    Ok(Response::new(pb::ListSourceRegistrationsResponse {
        registrations,
        next_page_token,
    }))
}
pub(super) async fn delete_source_registration(
    service: &EventModelServiceImpl,
    request: Request<pb::DeleteSourceRegistrationRequest>,
) -> Result<Response<pb::DeleteSourceRegistrationResponse>, Status> {
    let ctx = Context::new(&request, crate::auth::Role::Admin)?;
    let req = request.into_inner();
    let _lock = service.lock_mutations().await;
    let mut doc = load(service, &ctx, &req.registration_id, true).await?;
    config_guard(&doc, &req.if_match)?;
    ensure_settled(&doc.state)?;
    for run in doc.state.runs.iter().filter_map(|s| s.run.as_ref()) {
        service
            .store
            .source_expire_transaction(&doc.key, &run.id, &run.preview_digest)
            .await
            .map_err(store_err)?;
    }
    doc.state = SourceState {
        registration: doc.state.registration,
        deleted: true,
        ..Default::default()
    };
    save(service, &mut doc).await?;
    Ok(Response::new(pb::DeleteSourceRegistrationResponse {}))
}
fn validated(snapshot: &pb::SourceSnapshot) -> Result<bundle::ValidatedBundle, Status> {
    if !snapshot.bytes_available {
        return Err(Status::failed_precondition("source bytes expired"));
    }
    let files: Vec<_> = snapshot
        .files
        .iter()
        .map(|f| bundle::SuppliedFile {
            logical_name: &f.logical_name,
            media_type: &f.media_type,
            bytes: &f.content,
        })
        .collect();
    bundle::validate_bundle(&snapshot.inventory_bytes, &files).map_err(|_| invalid())
}
pub(super) async fn accept_source_snapshot(
    service: &EventModelServiceImpl,
    request: Request<pb::AcceptSourceSnapshotRequest>,
) -> Result<Response<pb::AcceptSourceSnapshotResponse>, Status> {
    let ctx = Context::new(&request, crate::auth::Role::Writer)?;
    let req = request.into_inner();
    let _lock = service.lock_mutations().await;
    let mut doc = load(service, &ctx, &req.registration_id, true).await?;
    config_guard(&doc, &req.registration_revision)?;
    ensure_settled(&doc.state)?;
    let registration = registration(&doc.state)?.clone();
    let files: Vec<_> = req
        .files
        .iter()
        .map(|f| bundle::SuppliedFile {
            logical_name: &f.logical_name,
            media_type: &f.media_type,
            bytes: &f.content,
        })
        .collect();
    let bundle = bundle::validate_bundle(&req.inventory_bytes, &files).map_err(|_| invalid())?;
    if bundle.inventory().registration().as_str() != registration.id {
        return Err(invalid());
    }
    let mut budget = bounded(core::RunBudget::new(
        registration.budgets.ok_or_else(invalid)?,
    ))?;
    bounded(budget.charge(
        core::BudgetWork::SuppliedBytes,
        (req.inventory_bytes.len() + req.files.iter().map(|f| f.content.len()).sum::<usize>())
            as u64,
    ))?;
    bounded(budget.charge(core::BudgetWork::Files, req.files.len() as u64))?;
    bounded(budget.charge(
        core::BudgetWork::Resources,
        bundle.inventory().resources().len() as u64,
    ))?;
    bounded(budget.charge(
        core::BudgetWork::Relations,
        bundle.inventory().relations().len() as u64,
    ))?;
    let source_version = bundle.inventory().source_version().as_str().to_owned();
    let bundle_digest = bundle.bundle_digest().hex();
    if let Some(old) = doc
        .state
        .snapshots
        .iter()
        .find(|s| s.source_version == source_version)
    {
        if old.bundle_digest != bundle_digest {
            return Err(Status::already_exists(
                "source version has different immutable bytes",
            ));
        }
        return Ok(Response::new(pb::AcceptSourceSnapshotResponse {
            snapshot: Some(snapshot_summary(old)),
        }));
    }
    let created = now();
    let snapshot = pb::SourceSnapshot {
        id: revision(),
        revision: revision(),
        registration_id: registration.id.clone(),
        source_version: source_version.clone(),
        inventory_bytes: req.inventory_bytes,
        files: bundle
            .files()
            .iter()
            .map(|f| pb::SourceFile {
                logical_name: f.logical_name().as_str().into(),
                media_type: f.media_type().as_str().into(),
                content: f.bytes().to_vec(),
                content_digest: f.digest().hex(),
            })
            .collect(),
        inventory_digest: bundle.inventory_digest().hex(),
        bundle_digest: bundle_digest.clone(),
        declared_coverage: if bundle.inventory().declared_coverage()
            == bundle::DeclaredCoverage::Complete
        {
            pb::SourceCoverage::Complete as i32
        } else {
            pb::SourceCoverage::Partial as i32
        },
        created_at_unix_ms: created,
        accepted_at_unix_ms: None,
        expires_at_unix_ms: created
            + (registration
                .retention
                .as_ref()
                .ok_or_else(invalid)?
                .unaccepted_snapshot_seconds
                .cast_signed())
                * 1000,
        bytes_available: true,
        profile_revision: 1,
    };
    doc.state.tombstones.push(pb::SourceIdentityTombstone {
        registration_id: registration.id,
        identity: Some(pb::source_identity_tombstone::Identity::SourceVersion(
            source_version,
        )),
        request_digest: bundle_digest,
        branch: None,
    });
    doc.state.overlays.push(StoredOverlay {
        snapshot_id: snapshot.id.clone(),
        entries: captured_overlay(&snapshot.registration_id, &bundle),
    });
    doc.state.snapshots.push(snapshot.clone());
    save(service, &mut doc).await?;
    Ok(Response::new(pb::AcceptSourceSnapshotResponse {
        snapshot: Some(snapshot_summary(&snapshot)),
    }))
}
pub(super) async fn get_source_snapshot(
    service: &EventModelServiceImpl,
    request: Request<pb::GetSourceSnapshotRequest>,
) -> Result<Response<pb::GetSourceSnapshotResponse>, Status> {
    let ctx = Context::new(&request, crate::auth::Role::Reader)?;
    let req = request.into_inner();
    let doc = load(service, &ctx, &req.registration_id, false).await?;
    let snapshot = doc
        .state
        .snapshots
        .iter()
        .find(|s| s.id == req.snapshot_id)
        .ok_or_else(missing)?;
    Ok(Response::new(pb::GetSourceSnapshotResponse {
        snapshot: Some(if req.include_bytes {
            snapshot.clone()
        } else {
            snapshot_summary(snapshot)
        }),
    }))
}
pub(super) async fn list_source_snapshots(
    service: &EventModelServiceImpl,
    request: Request<pb::ListSourceSnapshotsRequest>,
) -> Result<Response<pb::ListSourceSnapshotsResponse>, Status> {
    let ctx = Context::new(&request, crate::auth::Role::Reader)?;
    let req = request.into_inner();
    let doc = load(service, &ctx, &req.registration_id, false).await?;
    let budgets = registration(&doc.state)?
        .budgets
        .as_ref()
        .ok_or_else(invalid)?;
    let items: Vec<_> = doc.state.snapshots.iter().map(snapshot_summary).collect();
    let (snapshots, next_page_token) = page(
        &items,
        page_start(&req.page_token)?,
        page_limit(req.page_size, budgets.page_records)?,
        budgets.page_bytes,
    )?;
    Ok(Response::new(pb::ListSourceSnapshotsResponse {
        snapshots,
        next_page_token,
    }))
}
fn selection_owner(selection: &pb::SourceSelection) -> Result<pb::EntityRef, Status> {
    use pb::source_selection::Selection;
    match selection.selection.as_ref().ok_or_else(invalid)? {
        Selection::Entity(owner) => Ok(owner.clone()),
        Selection::Connection(r) => r.owner.clone().ok_or_else(invalid),
        Selection::ContractMember(r) => Ok(pb::EntityRef {
            kind: pb::EntityKind::Specification as i32,
            id: r.specification.as_ref().and_then(|s| s.id.clone()),
        }),
        Selection::ContractType(r) => Ok(pb::EntityRef {
            kind: pb::EntityKind::Specification as i32,
            id: r
                .schema
                .as_ref()
                .and_then(|r| r.specification.as_ref())
                .and_then(|s| s.id.clone()),
        }),
        Selection::DataPort(r) => Ok(pb::EntityRef {
            kind: pb::EntityKind::DataProduct as i32,
            id: r.product.as_ref().and_then(|s| s.id.clone()),
        }),
        Selection::Document(r) => {
            trogon_atlas_core::decision_document::document_owner(r).map_err(|_| invalid())
        }
    }
}
fn carries_slice_authoring(entity: &pb::Entity) -> bool {
    match &entity.kind {
        Some(pb::entity::Kind::DataProduct(product)) => !product.resource_associations.is_empty(),
        Some(pb::entity::Kind::Specification(specification)) => {
            !specification.message_channels.is_empty()
                || specification.messages.iter().any(|message| {
                    message.schemas.iter().any(|binding| {
                        !binding.environments.is_empty() || binding.is_default.is_some()
                    })
                })
        }
        _ => false,
    }
}

fn forbidden_native_selection(entity: &pb::Entity, path: &str) -> bool {
    // Connection occurrence paths are rooted at the entity oneof field
    // (`data_product.resource_associations[0]`), while the native guards
    // name the field on the kind body.
    let path = path.split_once('.').map_or(path, |(_, rest)| rest);
    match &entity.kind {
        Some(pb::entity::Kind::DataProduct(_)) => path.starts_with("resource_associations["),
        Some(pb::entity::Kind::Specification(specification)) => {
            if path.starts_with("message_channels[") {
                return true;
            }
            let Some(rest) = path.strip_prefix("messages[") else {
                return false;
            };
            let Some((message, rest)) = rest.split_once("].schemas[") else {
                return false;
            };
            let Some((binding, suffix)) = rest.split_once(']') else {
                return false;
            };
            if !(suffix.is_empty() || suffix.starts_with('.')) {
                return false;
            }
            let Ok(message) = message.parse::<usize>() else {
                return false;
            };
            let Ok(binding) = binding.parse::<usize>() else {
                return false;
            };
            specification
                .messages
                .get(message)
                .and_then(|message| message.schemas.get(binding))
                .is_some_and(|binding| {
                    !binding.environments.is_empty() || binding.is_default.is_some()
                })
        }
        _ => false,
    }
}

fn native_document_context_only(
    current: Option<&pb::Entity>,
    incoming: Option<&pb::Entity>,
    ownership: &pb::SourceFieldOwnership,
) -> Result<(), Status> {
    let has_extensions = |entity: &pb::Entity| {
        trogon_atlas_core::document_catalog_context::document_has_native_extensions(entity)
            .unwrap_or(false)
    };
    let whole_owner = matches!(
        ownership.policy,
        Some(pb::source_field_ownership::Policy::WholeOwner(_))
    );
    if (current.is_none() && incoming.is_some_and(has_extensions))
        || (whole_owner
            && (current.is_some_and(has_extensions) || incoming.is_some_and(has_extensions)))
    {
        return Err(Status::invalid_argument(
            "document catalog context supports native authoring only",
        ));
    }
    Ok(())
}

fn native_relationships_only(
    current: Option<&pb::Entity>,
    incoming: Option<&pb::Entity>,
    ownership: &pb::SourceFieldOwnership,
) -> Result<(), Status> {
    if let Some(incoming) = incoming {
        let unit = if matches!(
            ownership.policy,
            Some(pb::source_field_ownership::Policy::WholeOwner(_))
        ) {
            core::OwnershipUnit::WholeOwner
        } else {
            core::OwnershipUnit::Field(pb::SourceOwnedField::Title)
        };
        trogon_atlas_core::message_claims::validate_source_write(current, incoming, unit).map_err(
            |_| Status::invalid_argument("message claims support native authoring only"),
        )?;
    } else if matches!(
        ownership.policy,
        Some(pb::source_field_ownership::Policy::WholeOwner(_))
    ) && current.is_some_and(trogon_atlas_core::message_claims::has_content)
    {
        return Err(Status::invalid_argument(
            "message claims support native authoring only",
        ));
    }
    let has_native_authoring = |entity: &pb::Entity| -> Result<bool, Status> {
        Ok(
            matches!(&entity.kind, Some(pb::entity::Kind::Decision(value)) if value.authored_on.is_some() || value.decided_on.is_some() || !value.decision_makers.is_empty() || !value.amends.is_empty())
                || matches!(&entity.kind, Some(pb::entity::Kind::Term(term)) if term.owning_domain.is_some())
                || matches!(&entity.kind, Some(pb::entity::Kind::SystemGroup(v)) if !v.members.is_empty() || !v.relationships.is_empty() || !v.actors.is_empty())
                || matches!(&entity.kind, Some(pb::entity::Kind::Domain(v)) if !v.members.is_empty())
                || !trogon_atlas_core::architectural_relationships::relationships(entity)
                    .is_empty()
                || !trogon_atlas_core::communication_intent::declarations(entity).is_empty()
                || !trogon_atlas_core::channel_links::links(entity).is_empty()
                || !trogon_atlas_core::storage_claims::claims(entity).is_empty()
                || matches!(&entity.kind, Some(pb::entity::Kind::Component(value)) if !value.ui_hostings.is_empty())
                || trogon_atlas_core::design_topology::graph(entity)
                    .is_some_and(|graph| !graph.steps.is_empty() || !graph.entry_steps.is_empty())
                || trogon_atlas_core::agent_metadata::has_native_additions(entity)
                    .map_err(|_| invalid())?,
        )
    };
    let whole_owner = matches!(
        ownership.policy,
        Some(pb::source_field_ownership::Policy::WholeOwner(_))
    );
    let incoming_native = incoming
        .map(has_native_authoring)
        .transpose()?
        .unwrap_or(false);
    let current_native = current
        .map(has_native_authoring)
        .transpose()?
        .unwrap_or(false);
    if (current.is_none() && incoming_native)
        || (whole_owner && (current_native || incoming_native))
    {
        return Err(Status::invalid_argument(
            if trogon_atlas_core::design_topology::graph(incoming.or(current).ok_or_else(invalid)?)
                .is_some()
            {
                "design topology support native authoring only"
            } else {
                "selected extensions support native authoring only"
            },
        ));
    }
    if (current.is_none() && incoming.is_some_and(carries_slice_authoring))
        || (whole_owner
            && (current.is_some_and(carries_slice_authoring)
                || incoming.is_some_and(carries_slice_authoring)))
    {
        return Err(Status::invalid_argument(
            "resource associations, message channels, and schema variants support native authoring only",
        ));
    }
    Ok(())
}
fn native_business_data_only(
    current: Option<&pb::Entity>,
    incoming: Option<&pb::Entity>,
    ownership: &pb::SourceFieldOwnership,
) -> Result<(), Status> {
    let whole_owner = matches!(
        ownership.policy,
        Some(pb::source_field_ownership::Policy::WholeOwner(_))
    );
    if (current.is_none()
        && incoming.is_some_and(trogon_atlas_core::business_data::declares_business))
        || (whole_owner
            && (current.is_some_and(trogon_atlas_core::business_data::declares_business)
                || incoming.is_some_and(trogon_atlas_core::business_data::declares_business)))
    {
        return Err(Status::invalid_argument(
            "business declarations support native authoring only",
        ));
    }
    Ok(())
}

fn resolve_selection(selection: &pb::SourceSelection, entity: &pb::Entity) -> Result<(), Status> {
    use pb::source_selection::Selection;
    match selection.selection.as_ref().ok_or_else(invalid)? {
        Selection::Entity(r) => {
            if entity_kind(entity)? as i32 != r.kind || Some(entity_id(entity)?) != r.id.as_ref() {
                return Err(conflict());
            }
        }
        Selection::Connection(r) => {
            let occurrence = trogon_atlas_core::connections::resolve_connection(entity, r)
                .map_err(|_| conflict())?;
            if occurrence.path.contains(".decision_makers[") || occurrence.path.contains(".amends[")
            {
                return Err(Status::invalid_argument(
                    "decision makers and amendments support native authoring only",
                ));
            }
            if forbidden_native_selection(entity, &occurrence.path) {
                return Err(Status::invalid_argument(
                    "resource associations, message channels, and schema variants support native authoring only",
                ));
            }
        }
        Selection::ContractMember(r) => {
            trogon_atlas_core::specification::resolve_member(entity, r).map_err(|_| conflict())?;
        }
        Selection::ContractType(r) => {
            trogon_atlas_core::specification::resolve_type(entity, r).map_err(|_| conflict())?;
        }
        Selection::DataPort(r) => {
            trogon_atlas_core::data_product::resolve_port(entity, r).map_err(|_| conflict())?;
        }
        Selection::Document(r) => {
            trogon_atlas_core::decision_document::resolve_document_selection(r, entity)
                .map_err(|_| conflict())?;
        }
    }
    Ok(())
}
fn observed(
    owner: &pb::EntityRef,
    current: &trogon_atlas_store::StoredEntity,
) -> pb::SourceObservedOwner {
    pb::SourceObservedOwner {
        owner: Some(owner.clone()),
        uid: current
            .entity
            .system
            .as_ref()
            .map(|s| s.uid.clone())
            .unwrap_or_default(),
        etag: current.etag.clone(),
    }
}
async fn checked_selection(
    service: &EventModelServiceImpl,
    ctx: &Context,
    selection: &pb::SourceSelection,
) -> Result<trogon_atlas_store::StoredEntity, Status> {
    let owner = selection_owner(selection)?;
    let id = owner.id.as_ref().ok_or_else(invalid)?;
    service.require_visible(&ctx.vis, &id.namespace).await?;
    let current = service
        .store
        .get(
            pb::EntityKind::try_from(owner.kind).map_err(|_| invalid())?,
            id,
            ctx.branch.as_deref(),
        )
        .await
        .map_err(store_err)?;
    resolve_selection(selection, &current.entity)?;
    Ok(current)
}
async fn ownership_available(
    service: &EventModelServiceImpl,
    proposed: &pb::SourceMapping,
    own_registration: &str,
) -> Result<(), Status> {
    let selection = proposed.selection.as_ref().ok_or_else(invalid)?;
    let owner = selection_owner(selection)?;
    let policy = proposed.ownership.as_ref().ok_or_else(invalid)?;
    if matches!(
        policy.policy,
        Some(pb::source_field_ownership::Policy::None(_))
    ) {
        return Ok(());
    }
    let mut cursor = None;
    let mut examined = 0usize;
    loop {
        let roots = service
            .store
            .source_list(cursor.as_deref(), 500)
            .await
            .map_err(store_err)?;
        for key in roots.keys {
            let document = service
                .store
                .source_get(&key)
                .await
                .map_err(store_err)?
                .ok_or_else(missing)?;
            let state = SourceState::decode(document.bytes.as_slice()).map_err(|_| conflict())?;
            if state.deleted {
                continue;
            }
            for mapping in &state.mappings {
                examined += 1;
                if examined > 4096 {
                    return Err(Status::resource_exhausted(
                        "source ownership scan budget exhausted",
                    ));
                }
                if mapping.detached
                    || mapping.branch != proposed.branch
                    || (key.as_str() == own_registration && mapping.id == proposed.id)
                {
                    continue;
                }
                let Some(other) = mapping.selection.as_ref() else {
                    return Err(conflict());
                };
                if selection_owner(other)? != owner {
                    continue;
                }
                let other = mapping.ownership.as_ref().ok_or_else(invalid)?;
                let overlap = match (&policy.policy, &other.policy) {
                    (_, Some(pb::source_field_ownership::Policy::None(_))) => false,
                    (
                        Some(pb::source_field_ownership::Policy::Fields(a)),
                        Some(pb::source_field_ownership::Policy::Fields(b)),
                    ) => a.fields.iter().any(|f| b.fields.contains(f)),
                    _ => true,
                };
                if overlap {
                    return Err(Status::failed_precondition(
                        "native ownership unit is already claimed",
                    ));
                }
            }
        }
        cursor = roots.next_cursor;
        if cursor.is_none() {
            break;
        }
    }
    Ok(())
}
async fn prepare_mapping(
    service: &EventModelServiceImpl,
    ctx: &Context,
    registration: &pb::SourceRegistration,
    value: &mut pb::SourceMapping,
) -> Result<(), Status> {
    token(&value.id)?;
    let source = value.source.as_ref().ok_or_else(invalid)?;
    token(&source.category)?;
    token(&source.key)?;
    if source.registration_id != registration.id || value.branch != ctx.branch {
        return Err(invalid());
    }
    let selection = value.selection.as_ref().ok_or_else(invalid)?;
    let policy = value.ownership.as_ref().ok_or_else(invalid)?;
    bounded(core::validate_selection_ownership(selection, policy))?;
    let owner = selection_owner(selection)?;
    let id = owner.id.as_ref().ok_or_else(invalid)?;
    if !registration.destination_namespaces.contains(&id.namespace)
        && !matches!(
            policy.policy,
            Some(pb::source_field_ownership::Policy::None(_))
        )
    {
        return Err(Status::permission_denied(
            "source destination was not selected",
        ));
    }
    existing_namespace(service, ctx, &id.namespace).await?;
    if !matches!(
        policy.policy,
        Some(pb::source_field_ownership::Policy::None(_))
    ) {
        let mut targets = WriteTargets::none();
        targets.insert(&id.namespace);
        service
            .authorize_write(ctx.principal.as_ref(), ctx.branch.as_deref(), &targets)
            .await?;
    }
    match checked_selection(service, ctx, selection).await {
        Ok(current) => {
            native_relationships_only(Some(&current.entity), None, policy)?;
            native_document_context_only(Some(&current.entity), None, policy)?;
            native_business_data_only(Some(&current.entity), None, policy)?;
            bounded(core::validate_ownership(&current.entity, policy))?;
            let observation = observed(&owner, &current);
            let adoption = value.adoption.as_ref().ok_or_else(|| {
                Status::failed_precondition("explicit observed adoption is required")
            })?;
            if !adoption.authorized_adoption || adoption.observed.as_ref() != Some(&observation) {
                return Err(conflict());
            }
            value.observed = Some(observation.clone());
            value.base = Some(pb::SourceMergeBase {
                revision: revision(),
                snapshot_id: String::new(),
                source_version: String::new(),
                source_value: Some(current.entity.clone()),
                native_value: Some(current.entity),
                observed: Some(observation),
                branch: ctx.branch.clone(),
                ownership_policy_revision: registration.ownership_policy_revision.clone(),
                accepted_at_unix_ms: now(),
                expires_at_unix_ms: now()
                    + registration
                        .retention
                        .as_ref()
                        .ok_or_else(invalid)?
                        .base_seconds
                        .cast_signed()
                        * 1000,
                run_id: String::new(),
            });
        }
        Err(status) if status.code() == tonic::Code::NotFound => {
            if !matches!(
                selection.selection,
                Some(pb::source_selection::Selection::Entity(_))
            ) || value.adoption.is_some()
            {
                return Err(conflict());
            }
            value.observed = Some(pb::SourceObservedOwner {
                owner: Some(owner),
                uid: String::new(),
                etag: String::new(),
            });
            value.base = None;
        }
        Err(status) => return Err(status),
    }
    ownership_available(service, value, &registration.id).await?;
    value.revision = revision();
    value.detached = false;
    value.last_accepted_run_id.clear();
    Ok(())
}
pub(super) async fn create_source_mapping(
    service: &EventModelServiceImpl,
    request: Request<pb::CreateSourceMappingRequest>,
) -> Result<Response<pb::CreateSourceMappingResponse>, Status> {
    let ctx = Context::new(&request, crate::auth::Role::Writer)?;
    let req = request.into_inner();
    let mut value = req.mapping.ok_or_else(invalid)?;
    if value.base.is_some()
        || value.observed.is_some()
        || !value.revision.is_empty()
        || value.detached
        || !value.last_accepted_run_id.is_empty()
    {
        return Err(invalid());
    }
    let registration_id = value
        .source
        .as_ref()
        .ok_or_else(invalid)?
        .registration_id
        .clone();
    let _lock = service.lock_mutations().await;
    let mut doc = load(service, &ctx, &registration_id, true).await?;
    ensure_settled(&doc.state)?;
    config_guard(&doc, &req.registration_revision)?;
    if doc.state.mappings.iter().any(|m| {
        m.id == value.id || (!m.detached && m.source == value.source && m.branch == value.branch)
    }) {
        return Err(Status::already_exists("source mapping exists"));
    }
    if doc.state.mappings.len()
        >= registration(&doc.state)?
            .budgets
            .as_ref()
            .ok_or_else(invalid)?
            .mappings as usize
    {
        return Err(Status::resource_exhausted(
            "source mapping budget exhausted",
        ));
    }
    prepare_mapping(service, &ctx, registration(&doc.state)?, &mut value).await?;
    doc.state.mappings.push(value.clone());
    save(service, &mut doc).await?;
    Ok(Response::new(pb::CreateSourceMappingResponse {
        mapping: Some(value),
    }))
}
pub(super) async fn update_source_mapping(
    service: &EventModelServiceImpl,
    request: Request<pb::UpdateSourceMappingRequest>,
) -> Result<Response<pb::UpdateSourceMappingResponse>, Status> {
    let ctx = Context::new(&request, crate::auth::Role::Writer)?;
    let req = request.into_inner();
    let mut value = req.mapping.ok_or_else(invalid)?;
    let registration_id = value
        .source
        .as_ref()
        .ok_or_else(invalid)?
        .registration_id
        .clone();
    let _lock = service.lock_mutations().await;
    let mut doc = load(service, &ctx, &registration_id, true).await?;
    ensure_settled(&doc.state)?;
    config_guard(&doc, &req.registration_revision)?;
    let index = doc
        .state
        .mappings
        .iter()
        .position(|m| m.id == value.id)
        .ok_or_else(missing)?;
    let old = &doc.state.mappings[index];
    guard(&old.revision, &req.if_match)?;
    if value.base != old.base
        || value.observed != old.observed
        || value.last_accepted_run_id != old.last_accepted_run_id
        || value.source != old.source
        || value.branch != old.branch
        || value.detached
    {
        return Err(invalid());
    }
    prepare_mapping(service, &ctx, registration(&doc.state)?, &mut value).await?;
    doc.state.mappings[index] = value.clone();
    save(service, &mut doc).await?;
    Ok(Response::new(pb::UpdateSourceMappingResponse {
        mapping: Some(value),
    }))
}
async fn visible_mapping(
    service: &EventModelServiceImpl,
    ctx: &Context,
    value: &pb::SourceMapping,
) -> Result<(), Status> {
    if value.branch != ctx.branch {
        return Err(missing());
    }
    let owner = selection_owner(value.selection.as_ref().ok_or_else(invalid)?)?;
    service
        .require_visible(&ctx.vis, &owner.id.as_ref().ok_or_else(invalid)?.namespace)
        .await
}
pub(super) async fn get_source_mapping(
    service: &EventModelServiceImpl,
    request: Request<pb::GetSourceMappingRequest>,
) -> Result<Response<pb::GetSourceMappingResponse>, Status> {
    let ctx = Context::new(&request, crate::auth::Role::Reader)?;
    let req = request.into_inner();
    let doc = load(service, &ctx, &req.registration_id, false).await?;
    let value = doc
        .state
        .mappings
        .iter()
        .find(|m| m.id == req.mapping_id)
        .ok_or_else(missing)?;
    visible_mapping(service, &ctx, value).await?;
    Ok(Response::new(pb::GetSourceMappingResponse {
        mapping: Some(value.clone()),
    }))
}
pub(super) async fn list_source_mappings(
    service: &EventModelServiceImpl,
    request: Request<pb::ListSourceMappingsRequest>,
) -> Result<Response<pb::ListSourceMappingsResponse>, Status> {
    let ctx = Context::new(&request, crate::auth::Role::Reader)?;
    let req = request.into_inner();
    let doc = load(service, &ctx, &req.registration_id, false).await?;
    let budgets = registration(&doc.state)?
        .budgets
        .as_ref()
        .ok_or_else(invalid)?;
    let mut values = Vec::new();
    for value in &doc.state.mappings {
        if value.branch != ctx.branch {
            continue;
        }
        match visible_mapping(service, &ctx, value).await {
            Ok(()) => values.push(value.clone()),
            Err(status) if status.code() == tonic::Code::NotFound => {}
            Err(status) => return Err(status),
        }
    }
    let (mappings, next_page_token) = page(
        &values,
        page_start(&req.page_token)?,
        page_limit(req.page_size, budgets.page_records)?,
        budgets.page_bytes,
    )?;
    Ok(Response::new(pb::ListSourceMappingsResponse {
        mappings,
        next_page_token,
    }))
}
pub(super) async fn delete_source_mapping(
    service: &EventModelServiceImpl,
    request: Request<pb::DeleteSourceMappingRequest>,
) -> Result<Response<pb::DeleteSourceMappingResponse>, Status> {
    let ctx = Context::new(&request, crate::auth::Role::Writer)?;
    let req = request.into_inner();
    let _lock = service.lock_mutations().await;
    let mut doc = load(service, &ctx, &req.registration_id, true).await?;
    config_guard(&doc, &req.registration_revision)?;
    ensure_settled(&doc.state)?;
    let index = doc
        .state
        .mappings
        .iter()
        .position(|m| m.id == req.mapping_id)
        .ok_or_else(missing)?;
    visible_mapping(service, &ctx, &doc.state.mappings[index]).await?;
    guard(&doc.state.mappings[index].revision, &req.if_match)?;
    doc.state.mappings[index].detached = true;
    doc.state.mappings[index].revision = revision();
    save(service, &mut doc).await?;
    Ok(Response::new(pb::DeleteSourceMappingResponse {}))
}
fn source_key(registration: &str, key: &bundle::ResourceKey) -> pb::SourceResourceKey {
    pb::SourceResourceKey {
        registration_id: registration.into(),
        category: key.category().as_str().into(),
        key: key.key().as_str().into(),
    }
}
fn captured_overlay(
    registration: &str,
    supplied: &bundle::ValidatedBundle,
) -> Vec<pb::SourceOverlayEntry> {
    let mut entries = Vec::new();
    for resource in supplied.inventory().resources() {
        entries.push(pb::SourceOverlayEntry {
            source: Some(source_key(registration, resource.key())),
            pointer: Some(pb::SourcePointer {
                kind: resource.pointer().kind().as_str().into(),
                raw: resource.pointer().raw().as_str().into(),
            }),
            logical_name: resource.file().as_str().into(),
            status: pb::SourceOverlayStatus::Deferred as i32,
            reason_code: "explicit_mapping_required".into(),
            ..Default::default()
        });
    }
    for relation in supplied.inventory().relations() {
        entries.push(pb::SourceOverlayEntry {
            source: Some(source_key(registration, relation.owner())),
            occurrence_key: Some(relation.occurrence().as_str().into()),
            role: Some(relation.role().as_str().into()),
            position: Some(relation.position()),
            pointer: Some(pb::SourcePointer {
                kind: relation.target().kind().as_str().into(),
                raw: relation.target().raw().as_str().into(),
            }),
            logical_name: relation.file().as_str().into(),
            status: pb::SourceOverlayStatus::Deferred as i32,
            reason_code: "inert_source_pointer".into(),
            ..Default::default()
        });
    }
    entries
}
enum OverlaySelection {
    Present(Box<trogon_atlas_store::StoredEntity>),
    Missing,
    Unavailable,
}
async fn overlay_selection(
    service: &EventModelServiceImpl,
    ctx: &Context,
    selection: &pb::SourceSelection,
) -> Result<OverlaySelection, Status> {
    let owner = selection_owner(selection)?;
    let id = owner.id.as_ref().ok_or_else(invalid)?;
    match service.require_visible(&ctx.vis, &id.namespace).await {
        Ok(()) => {}
        Err(error) if error.code() == tonic::Code::NotFound => {
            return Ok(OverlaySelection::Unavailable);
        }
        Err(error) => return Err(error),
    }
    let current = match service
        .store
        .get(
            pb::EntityKind::try_from(owner.kind).map_err(|_| invalid())?,
            id,
            ctx.branch.as_deref(),
        )
        .await
    {
        Ok(current) => current,
        Err(StoreError::NotFound) => return Ok(OverlaySelection::Missing),
        Err(error) => return Err(store_err(error)),
    };
    match resolve_selection(selection, &current.entity) {
        Ok(()) => Ok(OverlaySelection::Present(Box::new(current))),
        Err(error)
            if matches!(
                error.code(),
                tonic::Code::NotFound | tonic::Code::FailedPrecondition
            ) =>
        {
            Ok(OverlaySelection::Unavailable)
        }
        Err(error) => Err(error),
    }
}
async fn overlay_entries(
    service: &EventModelServiceImpl,
    ctx: &Context,
    state: &SourceState,
    snapshot: &pb::SourceSnapshot,
) -> Result<Vec<pb::SourceOverlayEntry>, Status> {
    let retention = registration(state)?
        .retention
        .as_ref()
        .ok_or_else(invalid)?;
    let protected = state
        .runs
        .iter()
        .filter_map(|stored| stored.run.as_ref())
        .any(|run| run.recovery_required && run.snapshot_id == snapshot.id);
    if !protected && snapshot_deadline(snapshot, retention.overlay_seconds) <= now() {
        return Err(Status::failed_precondition(
            "source overlay retention expired",
        ));
    }
    let mut entries = match state
        .overlays
        .iter()
        .find(|overlay| overlay.snapshot_id == snapshot.id)
    {
        Some(overlay) => overlay.entries.clone(),
        None => captured_overlay(&snapshot.registration_id, &validated(snapshot)?),
    };
    for entry in &mut entries {
        if entry.occurrence_key.is_some() {
            continue;
        }
        let Some(mapping) = state.mappings.iter().find(|mapping| {
            !mapping.detached && mapping.source == entry.source && mapping.branch == ctx.branch
        }) else {
            continue;
        };
        let selection = mapping.selection.as_ref().ok_or_else(invalid)?;
        match overlay_selection(service, ctx, selection).await? {
            OverlaySelection::Present(current) => {
                if mapping.observed.as_ref().is_some_and(|observed| {
                    current
                        .entity
                        .system
                        .as_ref()
                        .is_some_and(|system| system.uid == observed.uid)
                }) {
                    entry.status = pb::SourceOverlayStatus::Resolved as i32;
                    entry.authorized_candidates.push(selection.clone());
                    entry.reason_code = "exact_selection_checked".into();
                } else {
                    entry.status = pb::SourceOverlayStatus::Conflict as i32;
                    entry.reason_code = "native_incarnation_changed".into();
                }
            }
            OverlaySelection::Missing => {
                entry.status = pb::SourceOverlayStatus::Missing as i32;
                entry.reason_code = "exact_owner_missing".into();
            }
            OverlaySelection::Unavailable => {
                entry.status = pb::SourceOverlayStatus::Unavailable as i32;
                entry.reason_code = "selection_unavailable".into();
            }
        }
    }
    Ok(entries)
}
pub(super) async fn get_source_overlay(
    service: &EventModelServiceImpl,
    request: Request<pb::GetSourceOverlayRequest>,
) -> Result<Response<pb::GetSourceOverlayResponse>, Status> {
    let ctx = Context::new(&request, crate::auth::Role::Reader)?;
    let req = request.into_inner();
    let doc = load(service, &ctx, &req.registration_id, false).await?;
    let snapshot = doc
        .state
        .snapshots
        .iter()
        .find(|s| s.id == req.snapshot_id)
        .ok_or_else(missing)?;
    let entries = overlay_entries(service, &ctx, &doc.state, snapshot).await?;
    let incomplete = entries
        .iter()
        .any(|e| e.status != pb::SourceOverlayStatus::Resolved as i32);
    let budgets = registration(&doc.state)?
        .budgets
        .as_ref()
        .ok_or_else(invalid)?;
    let (entries, next_page_token) = page(
        &entries,
        page_start(&req.page_token)?,
        page_limit(req.page_size, budgets.page_records)?,
        budgets.page_bytes,
    )?;
    Ok(Response::new(pb::GetSourceOverlayResponse {
        overlay: Some(pb::SourceOverlay {
            snapshot_id: req.snapshot_id,
            revision: digest(snapshot),
            branch: ctx.branch,
            entries,
            incomplete,
        }),
        next_page_token,
    }))
}
fn native_guard(
    owner: &pb::EntityRef,
    current: Option<&trogon_atlas_store::StoredEntity>,
) -> pb::SourceNativeGuard {
    pb::SourceNativeGuard {
        owner: Some(owner.clone()),
        condition: Some(match current {
            Some(current) => pb::source_native_guard::Condition::Existing(observed(owner, current)),
            None => pb::source_native_guard::Condition::Absent(true),
        }),
    }
}
fn result(
    source: &pb::SourceResourceKey,
    mapping: &pb::SourceMapping,
    disposition: pb::SourceResultDisposition,
    reason: &str,
    field: Option<pb::SourceOwnedField>,
) -> pb::SourceRunResult {
    pb::SourceRunResult {
        source: Some(source.clone()),
        mapping_id: mapping.id.clone(),
        selection: mapping.selection.clone(),
        disposition: disposition as i32,
        reason_code: reason.into(),
        field: field.map(|f| f as i32),
    }
}
fn batch_request(
    plan: &pb::SourceRunPlan,
    validate_only: bool,
) -> Result<pb::BatchMutateRequest, Status> {
    let mut ops = Vec::new();
    for operation in &plan.operations {
        let op = match operation.operation.as_ref().ok_or_else(invalid)? {
            pb::source_native_operation::Operation::Put(put) => {
                let condition = put
                    .guard
                    .as_ref()
                    .and_then(|g| g.condition.as_ref())
                    .ok_or_else(invalid)?;
                pb::batch_mutate_op::Op::Put(pb::PutEntityRequest {
                    entity: put.entity.clone(),
                    create_only: matches!(
                        condition,
                        pb::source_native_guard::Condition::Absent(true)
                    ),
                    if_match: match condition {
                        pb::source_native_guard::Condition::Existing(o) => o.etag.clone(),
                        pb::source_native_guard::Condition::Absent(_) => String::new(),
                    },
                    ..Default::default()
                })
            }
            pb::source_native_operation::Operation::Delete(delete) => {
                let guard = delete.guard.as_ref().ok_or_else(invalid)?;
                let owner = guard.owner.as_ref().ok_or_else(invalid)?;
                let Some(pb::source_native_guard::Condition::Existing(observed)) = &guard.condition
                else {
                    return Err(invalid());
                };
                pb::batch_mutate_op::Op::Delete(pb::DeleteEntityRequest {
                    kind: owner.kind,
                    id: owner.id.clone(),
                    mode: pb::delete_entity_request::Mode::FailIfReferenced as i32,
                    if_match: observed.etag.clone(),
                    operation_id: String::new(),
                })
            }
        };
        ops.push(pb::BatchMutateOp { op: Some(op) });
    }
    Ok(pb::BatchMutateRequest {
        ops,
        validate_only,
        operation_id: String::new(),
    })
}
fn partition_equal(a: &bundle::InventoryPartition, b: &bundle::InventoryPartition) -> bool {
    a.categories() == b.categories()
        && a.logical_roots() == b.logical_roots()
        && a.selected_keys() == b.selected_keys()
}
fn canonical_entries<M: Message>(
    name: &str,
    message: &M,
    field: &str,
    entries: Vec<serde_json::Value>,
    branch: Option<&str>,
) -> Result<String, Status> {
    let mut json =
        trogon_atlas_core::transcode::message_json_value(name, message).map_err(|_| invalid())?;
    if !entries.is_empty() {
        let descriptor =
            trogon_atlas_core::transcode::message_descriptor(name).map_err(|_| invalid())?;
        let field = descriptor.get_field_by_name(field).ok_or_else(invalid)?;
        json.as_object_mut()
            .ok_or_else(invalid)?
            .insert(field.json_name().into(), serde_json::Value::Array(entries));
    }
    trogon_atlas_core::operation_id::operation_digest(name, branch, &json).map_err(|_| invalid())
}
async fn native_json<M: Message>(
    service: &EventModelServiceImpl,
    ctx: &Context,
    name: &str,
    message: &M,
    entity: Option<&pb::Entity>,
) -> Result<serde_json::Value, Status> {
    match entity {
        Some(entity) => {
            let namespace = &entity_id(entity)?.namespace;
            service.require_visible(&ctx.vis, namespace).await?;
            service
                .transcode_pool(namespace, ctx.branch.as_deref())
                .await?
                .message_json_value(name, message)
                .map_err(|_| invalid())
        }
        None => {
            trogon_atlas_core::transcode::message_json_value(name, message).map_err(|_| invalid())
        }
    }
}
async fn request_digest(
    service: &EventModelServiceImpl,
    ctx: &Context,
    request: &pb::PreviewSourceRunRequest,
) -> Result<String, Status> {
    let mut stripped = request.clone();
    stripped.native_payloads.clear();
    let mut entries = Vec::new();
    for payload in &request.native_payloads {
        let entity = payload.entity.as_ref().ok_or_else(invalid)?;
        entries.push(
            native_json(
                service,
                ctx,
                "trogonatlas.api.eventmodel.v1alpha1.SourceNativePayload",
                payload,
                Some(entity),
            )
            .await?,
        );
    }
    canonical_entries(
        "trogonatlas.api.eventmodel.v1alpha1.PreviewSourceRunRequest",
        &stripped,
        "native_payloads",
        entries,
        ctx.branch.as_deref(),
    )
}
async fn plan_digest(
    service: &EventModelServiceImpl,
    ctx: &Context,
    plan: &pb::SourceRunPlan,
) -> Result<String, Status> {
    let mut stripped = plan.clone();
    stripped.operations.clear();
    let mut entries = Vec::new();
    for operation in &plan.operations {
        let entity = match &operation.operation {
            Some(pb::source_native_operation::Operation::Put(put)) => {
                Some(put.entity.as_ref().ok_or_else(invalid)?)
            }
            Some(pb::source_native_operation::Operation::Delete(_)) => None,
            None => return Err(invalid()),
        };
        entries.push(
            native_json(
                service,
                ctx,
                "trogonatlas.api.eventmodel.v1alpha1.SourceNativeOperation",
                operation,
                entity,
            )
            .await?,
        );
    }
    canonical_entries(
        "trogonatlas.api.eventmodel.v1alpha1.SourceRunPlan",
        &stripped,
        "operations",
        entries,
        ctx.branch.as_deref(),
    )
}
fn field(unit: core::OwnershipUnit) -> Option<pb::SourceOwnedField> {
    match unit {
        core::OwnershipUnit::WholeOwner => None,
        core::OwnershipUnit::Field(f) => Some(f),
    }
}
enum ContextualMergeError {
    Source(core::SourceError),
    Status(Status),
}
impl From<core::SourceError> for ContextualMergeError {
    fn from(error: core::SourceError) -> Self {
        Self::Source(error)
    }
}
pub(super) async fn preview_source_run(
    service: &EventModelServiceImpl,
    request: Request<pb::PreviewSourceRunRequest>,
) -> Result<Response<pb::PreviewSourceRunResponse>, Status> {
    let started = tokio::time::Instant::now();
    let ceiling = std::time::Duration::from_millis(core::profile_budgets().phase_timeout_ms);
    tokio::time::timeout_at(
        started + ceiling,
        preview_source_run_attempt(service, request, started),
    )
    .await
    .map_err(|_| Status::deadline_exceeded("source attempt deadline exceeded"))?
}
async fn preview_source_run_attempt(
    service: &EventModelServiceImpl,
    request: Request<pb::PreviewSourceRunRequest>,
    started: tokio::time::Instant,
) -> Result<Response<pb::PreviewSourceRunResponse>, Status> {
    use pb::event_model_service_server::EventModelService;
    let ctx = Context::new(&request, crate::auth::Role::Writer)?;
    let req = request.into_inner();
    token(&req.run_id)?;
    let initial = load(service, &ctx, &req.registration_id, true).await?;
    let deadline = started
        + std::time::Duration::from_millis(
            registration(&initial.state)?
                .budgets
                .as_ref()
                .ok_or_else(invalid)?
                .phase_timeout_ms,
        );
    tokio::time::timeout_at(deadline, async move {
        let lock = service.lock_mutations().await;
        let mut doc = load(service, &ctx, &req.registration_id, true).await?;
        config_guard(&doc, &req.registration_revision)?;
        let request_digest = request_digest(service, &ctx, &req).await?;
        if let Some(existing) = doc
            .state
            .runs
            .iter()
            .filter_map(|r| r.run.as_ref())
            .find(|r| r.id == req.run_id)
        {
            if existing.request_digest != request_digest || existing.branch != ctx.branch {
                return Err(Status::already_exists(
                    "source run identity has different inputs",
                ));
            }
            authorize_run(service, &ctx, existing).await?;
            return Ok(Response::new(pb::PreviewSourceRunResponse {
                run: Some(summary(existing)),
            }));
        }
        ensure_settled(&doc.state)?;
        let registration = registration(&doc.state)?.clone();
        let snapshot = doc
            .state
            .snapshots
            .iter()
            .find(|s| s.id == req.snapshot_id)
            .ok_or_else(missing)?
            .clone();
        let supplied = validated(&snapshot)?;
        let mut budget = bounded(core::RunBudget::new(
            registration.budgets.ok_or_else(invalid)?,
        ))?;
        bounded(budget.charge(core::BudgetWork::Mappings, doc.state.mappings.len() as u64))?;
        bounded(
            budget.charge(
                core::BudgetWork::SuppliedBytes,
                (snapshot.inventory_bytes.len()
                    + snapshot
                        .files
                        .iter()
                        .map(|f| f.content.len())
                        .sum::<usize>()) as u64,
            ),
        )?;
        bounded(budget.charge(core::BudgetWork::Files, snapshot.files.len() as u64))?;
        bounded(budget.charge(
            core::BudgetWork::Resources,
            supplied.inventory().resources().len() as u64,
        ))?;
        bounded(budget.charge(
            core::BudgetWork::Relations,
            supplied.inventory().relations().len() as u64,
        ))?;
        bounded(budget.charge(
            core::BudgetWork::Examinations,
            supplied.inventory().relations().len() as u64,
        ))?;
        let mut charged_merge_results = 0u64;
        let mut plan = pb::SourceRunPlan {
            registration_revision: registration.revision.clone(),
            branch: ctx.branch.clone(),
            completeness: req.completeness.clone(),
            ..Default::default()
        };
        if let Some(branch) = &ctx.branch {
            plan.branch_revision = digest_bytes(
                service
                    .require_branch(branch)
                    .await?
                    .fork_changeset_id
                    .unwrap_or_default()
                    .as_bytes(),
            );
        }
        let mut results = Vec::new();
        let mut values = Vec::new();
        let mut fidelity = pb::SourceFidelityReport {
            checked_coverage: pb::SourceCoverage::Partial as i32,
            incomplete: false,
            native_reapply_supported: !req.overlay_only,
            original_bytes_recoverable: true,
            branch: ctx.branch.clone(),
            checked_namespaces: registration.destination_namespaces.clone(),
            bundle_digest: snapshot.bundle_digest.clone(),
            ..Default::default()
        };
        let graph = crate::graph::load_all(&service.store, ctx.branch.as_deref()).await?;
        if graph.truncated {
            return Err(Status::resource_exhausted(
                "complete native guard view exceeds available bounds",
            ));
        }
        let graph = service.narrow(Arc::new(graph.entities), &ctx.vis).await?;
        bounded(budget.charge(core::BudgetWork::Owners, graph.len() as u64))?;
        for current in graph.iter() {
            let owner = pb::EntityRef {
                kind: entity_kind(&current.entity)? as i32,
                id: Some(entity_id(&current.entity)?.clone()),
            };
            plan.native_guards.push(native_guard(&owner, Some(current)));
            fidelity.checked_owners.push(observed(&owner, current));
        }
        let inventory: BTreeSet<_> = supplied
            .inventory()
            .resources()
            .iter()
            .map(|r| {
                (
                    r.key().category().as_str().to_owned(),
                    r.key().key().as_str().to_owned(),
                )
            })
            .collect();
        let mut payload_keys = BTreeSet::new();
        for payload in &req.native_payloads {
            let source = payload.source.as_ref().ok_or_else(invalid)?;
            let incoming = payload.entity.as_ref().ok_or_else(invalid)?;
            if source.registration_id != registration.id
                || !inventory.contains(&(source.category.clone(), source.key.clone()))
                || !payload_keys.insert((source.category.clone(), source.key.clone()))
            {
                return Err(invalid());
            }
            crate::conv::validate_entity_content(incoming)?;
            if trogon_atlas_core::message_claims::has_content(incoming) {
                let id = entity_id(incoming)?;
                service.require_visible(&ctx.vis, &id.namespace).await?;
                let previous = match service
                    .store
                    .get(entity_kind(incoming)?, id, ctx.branch.as_deref())
                    .await
                {
                    Ok(row) => Some(row.entity),
                    Err(StoreError::NotFound) => None,
                    Err(e) => return Err(store_err(e)),
                };
                trogon_atlas_core::message_claims::validate_source_write(
                    previous.as_ref(),
                    incoming,
                    core::OwnershipUnit::Field(pb::SourceOwnedField::Title),
                )
                .map_err(|_| {
                    Status::invalid_argument("message claims support native authoring only")
                })?;
            }
            let Some(mapping) = doc.state.mappings.iter().find(|m| {
                !m.detached && m.source.as_ref() == Some(source) && m.branch == ctx.branch
            }) else {
                results.push(pb::SourceRunResult {
                    source: Some(source.clone()),
                    disposition: pb::SourceResultDisposition::Deferred as i32,
                    reason_code: "explicit_mapping_required".into(),
                    ..Default::default()
                });
                fidelity.incomplete = true;
                continue;
            };
            visible_mapping(service, &ctx, mapping).await?;
            let selection = mapping.selection.as_ref().ok_or_else(invalid)?;
            let ownership = mapping.ownership.as_ref().ok_or_else(invalid)?;
            bounded(core::validate_selection_ownership(selection, ownership))?;
            let owner = selection_owner(selection)?;
            if owner.kind != entity_kind(incoming)? as i32
                || owner.id.as_ref() != Some(entity_id(incoming)?)
            {
                return Err(invalid());
            }
            let kind = entity_kind(incoming)?;
            let id = entity_id(incoming)?;
            let current = match service.store.get(kind, id, ctx.branch.as_deref()).await {
                Ok(current) => Some(current),
                Err(StoreError::NotFound) => None,
                Err(e) => return Err(store_err(e)),
            };
            native_relationships_only(
                current.as_ref().map(|value| &value.entity),
                Some(incoming),
                ownership,
            )?;
            native_document_context_only(
                current.as_ref().map(|value| &value.entity),
                Some(incoming),
                ownership,
            )?;
            native_business_data_only(
                current.as_ref().map(|value| &value.entity),
                Some(incoming),
                ownership,
            )?;
            let guard = native_guard(&owner, current.as_ref());
            if !plan.native_guards.contains(&guard) {
                plan.native_guards.push(guard.clone());
            }
            plan.mapping_guards.push(pb::SourceMappingGuard {
                mapping_id: mapping.id.clone(),
                revision: mapping.revision.clone(),
                base_revision: mapping
                    .base
                    .as_ref()
                    .map(|b| b.revision.clone())
                    .unwrap_or_default(),
            });
            let mut proposed = None;
            if req.overlay_only
                || matches!(
                    ownership.policy,
                    Some(pb::source_field_ownership::Policy::None(_))
                )
            {
                results.push(result(
                    source,
                    mapping,
                    pb::SourceResultDisposition::Deferred,
                    "overlay_only",
                    None,
                ));
                fidelity.incomplete = true;
                continue;
            }
            ownership_available(service, mapping, &registration.id).await?;
            let mut targets = WriteTargets::none();
            targets.insert(&id.namespace);
            service
                .authorize_write(ctx.principal.as_ref(), ctx.branch.as_deref(), &targets)
                .await?;
            if !registration.destination_namespaces.contains(&id.namespace) {
                return Err(Status::permission_denied(
                    "source destination no longer selected",
                ));
            }
            match (&mapping.base, &current) {
                (None, None) => {
                    bounded(core::validate_create_ownership(incoming, ownership))?;
                    proposed = Some(incoming.clone());
                    results.push(result(
                        source,
                        mapping,
                        pb::SourceResultDisposition::Create,
                        "guarded_create",
                        None,
                    ));
                }
                (Some(base), Some(current)) => {
                    if base.expires_at_unix_ms <= now()
                        || base.source_value.is_none()
                        || base.native_value.is_none()
                    {
                        return Err(Status::failed_precondition(
                            "source base expired; explicit re-adoption is required",
                        ));
                    }
                    if base.branch != ctx.branch
                        || base.ownership_policy_revision != registration.ownership_policy_revision
                        || base.observed.as_ref().is_none_or(|o| {
                            current
                                .entity
                                .system
                                .as_ref()
                                .is_none_or(|s| s.uid != o.uid)
                        })
                    {
                        results.push(result(
                            source,
                            mapping,
                            pb::SourceResultDisposition::Conflict,
                            "native_incarnation_or_policy_changed",
                            None,
                        ));
                        continue;
                    }
                    resolve_selection(selection, &current.entity)?;
                    let types = service
                        .transcode_pool(&id.namespace, ctx.branch.as_deref())
                        .await?;
                    let merged = core::plan_merge_with_comparator(
                        base.source_value.as_ref().ok_or_else(invalid)?,
                        base.native_value.as_ref().ok_or_else(invalid)?,
                        incoming,
                        &current.entity,
                        ownership,
                        &mut budget,
                        |a, b| {
                            use prost_reflect::ReflectMessage;
                            let render = |value: &core::OwnedValue| {
                                types
                                    .message_json_value(
                                        value.message().descriptor().full_name(),
                                        value.message(),
                                    )
                                    .map_err(|error| {
                                        ContextualMergeError::Status(Status::invalid_argument(
                                            format!(
                                                "owned source type cannot be resolved: {error}"
                                            ),
                                        ))
                                    })
                            };
                            Ok::<_, ContextualMergeError>(render(a)? == render(b)?)
                        },
                    )
                    .map_err(|error| match error {
                        ContextualMergeError::Source(error) => source_error(error),
                        ContextualMergeError::Status(status) => status,
                    })?;
                    charged_merge_results += merged.decisions.len() as u64;
                    let mut update = false;
                    for decision in &merged.decisions {
                        let (disposition, reason) = match decision.disposition {
                            core::MergeDisposition::Unchanged => {
                                (pb::SourceResultDisposition::Unchanged, "unchanged")
                            }
                            core::MergeDisposition::Update => {
                                update = true;
                                (pb::SourceResultDisposition::Update, "source_owned_change")
                            }
                            core::MergeDisposition::LocalDrift => (
                                pb::SourceResultDisposition::LocalDrift,
                                "native_local_drift",
                            ),
                            core::MergeDisposition::Convergent => {
                                (pb::SourceResultDisposition::Convergent, "convergent")
                            }
                            core::MergeDisposition::Conflict => {
                                (pb::SourceResultDisposition::Conflict, "both_sides_changed")
                            }
                        };
                        results.push(result(
                            source,
                            mapping,
                            disposition,
                            reason,
                            field(decision.unit),
                        ));
                    }
                    if update && !merged.has_conflicts() {
                        proposed = Some(merged.proposed);
                    }
                }
                _ => {
                    results.push(result(
                        source,
                        mapping,
                        pb::SourceResultDisposition::Conflict,
                        "native_presence_changed",
                        None,
                    ));
                    continue;
                }
            }
            values.push(pb::SourceCommittedValue {
                source: Some(source.clone()),
                mapping_id: mapping.id.clone(),
                observed: Some(current.as_ref().map_or(
                    pb::SourceObservedOwner {
                        owner: Some(owner),
                        uid: String::new(),
                        etag: String::new(),
                    },
                    |c| observed(guard.owner.as_ref().unwrap_or(&pb::EntityRef::default()), c),
                )),
                entity: current.as_ref().map(|c| c.entity.clone()),
                deleted: false,
            });
            if let Some(entity) = proposed {
                trogon_atlas_core::business_data::reject_source_local_paths(&entity)
                    .map_err(|error| Status::invalid_argument(error.to_string()))?;
                plan.operations.push(pb::SourceNativeOperation {
                    mapping_id: mapping.id.clone(),
                    source: Some(source.clone()),
                    operation: Some(pb::source_native_operation::Operation::Put(
                        pb::SourcePutOperation {
                            entity: Some(entity),
                            guard: Some(guard),
                        },
                    )),
                });
            }
        }
        for resource in supplied.inventory().resources() {
            if payload_keys.contains(&(
                resource.key().category().as_str().to_owned(),
                resource.key().key().as_str().to_owned(),
            )) {
                continue;
            }
            results.push(pb::SourceRunResult {
                source: Some(source_key(&registration.id, resource.key())),
                disposition: pb::SourceResultDisposition::Deferred as i32,
                reason_code: "native_payload_not_captured".into(),
                ..Default::default()
            });
            fidelity.incomplete = true;
        }
        if req
            .completeness
            .as_ref()
            .is_some_and(|c| c.verified_complete)
        {
            let decision = req.completeness.as_ref().ok_or_else(invalid)?;
            if ctx
                .principal
                .as_ref()
                .is_some_and(|p| p.role < crate::auth::Role::Admin)
                || decision.inventory_digest != snapshot.inventory_digest
                || snapshot.declared_coverage != pb::SourceCoverage::Complete as i32
            {
                return Err(Status::permission_denied(
                "complete inventory decision is not authorized or does not match supplied bytes",
            ));
            }
            let base_snapshot = doc
                .state
                .snapshots
                .iter()
                .find(|s| s.id == decision.base_snapshot_id)
                .ok_or_else(|| {
                    Status::failed_precondition(
                        "complete inventory requires a retained partition base",
                    )
                })?;
            let base_bundle = validated(base_snapshot)?;
            if !partition_equal(
                supplied.inventory().partition(),
                base_bundle.inventory().partition(),
            ) {
                return Err(conflict());
            }
            fidelity.checked_coverage = pb::SourceCoverage::Complete as i32;
            for mapping in &doc.state.mappings {
                if mapping.detached || mapping.branch != ctx.branch {
                    continue;
                }
                let source = mapping.source.as_ref().ok_or_else(invalid)?;
                if inventory.contains(&(source.category.clone(), source.key.clone()))
                    || mapping
                        .base
                        .as_ref()
                        .is_none_or(|b| b.snapshot_id != base_snapshot.id)
                {
                    continue;
                }
                visible_mapping(service, &ctx, mapping).await?;
                let owner = selection_owner(mapping.selection.as_ref().ok_or_else(invalid)?)?;
                let id = owner.id.as_ref().ok_or_else(invalid)?;
                let current = service
                    .store
                    .get(
                        pb::EntityKind::try_from(owner.kind).map_err(|_| invalid())?,
                        id,
                        ctx.branch.as_deref(),
                    )
                    .await
                    .map_err(store_err)?;
                native_relationships_only(
                    Some(&current.entity),
                    None,
                    mapping.ownership.as_ref().ok_or_else(invalid)?,
                )?;
                native_document_context_only(
                    Some(&current.entity),
                    None,
                    mapping.ownership.as_ref().ok_or_else(invalid)?,
                )?;
                native_business_data_only(
                    Some(&current.entity),
                    None,
                    mapping.ownership.as_ref().ok_or_else(invalid)?,
                )?;
                let guard = native_guard(&owner, Some(&current));
                if !plan.native_guards.contains(&guard) {
                    plan.native_guards.push(guard.clone());
                }
                plan.mapping_guards.push(pb::SourceMappingGuard {
                    mapping_id: mapping.id.clone(),
                    revision: mapping.revision.clone(),
                    base_revision: mapping.base.as_ref().ok_or_else(invalid)?.revision.clone(),
                });
                if registration.deletion_policy
                    == pb::SourceDeletionPolicy::ExclusiveSafeDelete as i32
                    && matches!(
                        mapping.ownership.as_ref().and_then(|o| o.policy.as_ref()),
                        Some(pb::source_field_ownership::Policy::WholeOwner(_))
                    )
                {
                    let base = mapping.base.as_ref().ok_or_else(invalid)?;
                    if base.expires_at_unix_ms <= now()
                        || base.observed.as_ref().is_none_or(|o| {
                            current
                                .entity
                                .system
                                .as_ref()
                                .is_none_or(|s| s.uid != o.uid)
                        })
                        || base.native_value.as_ref().is_none_or(|e| {
                            !trogon_atlas_core::semantically_equal(e, &current.entity)
                        })
                    {
                        results.push(result(
                            source,
                            mapping,
                            pb::SourceResultDisposition::Conflict,
                            "deletion_base_or_native_drift",
                            None,
                        ));
                        continue;
                    }
                    plan.operations.push(pb::SourceNativeOperation {
                        mapping_id: mapping.id.clone(),
                        source: Some(source.clone()),
                        operation: Some(pb::source_native_operation::Operation::Delete(
                            pb::SourceDeleteOperation { guard: Some(guard) },
                        )),
                    });
                    values.push(pb::SourceCommittedValue {
                        source: Some(source.clone()),
                        mapping_id: mapping.id.clone(),
                        observed: Some(observed(&owner, &current)),
                        entity: None,
                        deleted: true,
                    });
                    results.push(result(
                        source,
                        mapping,
                        pb::SourceResultDisposition::Delete,
                        "exclusive_complete_partition_delete",
                        None,
                    ));
                } else {
                    results.push(result(
                        source,
                        mapping,
                        pb::SourceResultDisposition::Detach,
                        "complete_partition_mapping_retirement",
                        None,
                    ));
                }
            }
        }
        bounded(budget.charge(
            core::BudgetWork::NativeOperations,
            plan.operations.len() as u64,
        ))?;
        bounded(
            budget.charge(
                core::BudgetWork::ResultRecords,
                (results.len() as u64)
                    .checked_sub(charged_merge_results)
                    .ok_or_else(invalid)?,
            ),
        )?;
        bounded(budget.charge(
            core::BudgetWork::MappingUpdates,
            plan.mapping_guards.len() as u64,
        ))?;
        let conflicts = results
            .iter()
            .any(|r| r.disposition == pb::SourceResultDisposition::Conflict as i32);
        for value in &results {
            fidelity.records.push(pb::SourceFidelityRecord {
                source: value.source.clone(),
                selection: value.selection.clone(),
                field: value.field,
                disposition: if value.disposition == pb::SourceResultDisposition::Conflict as i32 {
                    pb::SourceFidelityDisposition::Conflicted as i32
                } else if value.disposition == pb::SourceResultDisposition::Deferred as i32 {
                    pb::SourceFidelityDisposition::RetainedOverlayOnly as i32
                } else {
                    pb::SourceFidelityDisposition::Preserved as i32
                },
                reason_code: value.reason_code.clone(),
                native_reapply_supported: !conflicts
                    && !req.overlay_only
                    && value.disposition != pb::SourceResultDisposition::Deferred as i32,
                original_bytes_recoverable: true,
                ..Default::default()
            });
        }
        for relation in supplied.inventory().relations() {
            fidelity.records.push(pb::SourceFidelityRecord {
                source: Some(source_key(&snapshot.registration_id, relation.owner())),
                occurrence_key: Some(relation.occurrence().as_str().into()),
                disposition: pb::SourceFidelityDisposition::RetainedOverlayOnly as i32,
                reason_code: "inert_source_relation_retained".into(),
                native_reapply_supported: false,
                original_bytes_recoverable: true,
                ..Default::default()
            });
        }
        if !supplied.inventory().relations().is_empty() {
            fidelity.incomplete = true;
            fidelity.native_reapply_supported = false;
        }
        fidelity.native_reapply_supported &= !fidelity.incomplete && !conflicts;
        let incomplete = conflicts || fidelity.incomplete;
        let report_bytes =
            fidelity.encoded_len() + results.iter().map(Message::encoded_len).sum::<usize>();
        bounded(budget.charge(core::BudgetWork::ResultBytes, report_bytes as u64))?;
        bounded(budget.charge(
            core::BudgetWork::ResultRecords,
            fidelity.records.len() as u64 + fidelity.checked_owners.len() as u64,
        ))?;
        drop(lock);
        if !conflicts && !req.overlay_only {
            let mut preview = ctx.request(batch_request(&plan, true)?);
            preview.extensions_mut().insert(SourceBusinessBypass);
            let validation = service.batch_mutate(preview).await?.into_inner();
            if validation.status != pb::batch_mutate_response::Status::Validated as i32 {
                return Err(Status::failed_precondition(
                    "native source plan does not pass admission",
                ));
            }
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(Status::deadline_exceeded(
                "source attempt deadline exceeded",
            ));
        }
        bounded(
            budget.check_elapsed(u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)),
        )?;
        let result_count = u32::try_from(results.len()).map_err(|_| invalid())?;
        let operation_count = u32::try_from(plan.operations.len()).map_err(|_| invalid())?;
        let created = now();
        let preview_digest = plan_digest(service, &ctx, &plan).await?;
        let run = pb::ReconciliationRun {
            result_count,
            operation_count,
            id: req.run_id.clone(),
            revision: revision(),
            registration_id: registration.id.clone(),
            snapshot_id: snapshot.id,
            request_digest: request_digest.clone(),
            configuration_digest: digest(&registration),
            preview_digest,
            status: pb::SourceRunStatus::Previewed as i32,
            plan: Some(plan),
            results,
            fidelity: Some(fidelity),
            budgets: registration.budgets,
            cancellation: pb::SourceCancellationDisposition::NotRequested as i32,
            created_at_unix_ms: created,
            expires_at_unix_ms: created
                + registration
                    .retention
                    .as_ref()
                    .ok_or_else(invalid)?
                    .preview_seconds
                    .cast_signed()
                    * 1000,
            overlay_only: req.overlay_only,
            incomplete,
            branch: ctx.branch.clone(),
            authorized_scope_digest: digest_bytes(
                registration.destination_namespaces.join("\0").as_bytes(),
            ),
            principal: ctx
                .principal
                .as_ref()
                .map_or_else(|| "anonymous".into(), |p| p.name.to_string()),
            ..Default::default()
        };
        let _lock = service.lock_mutations().await;
        doc.state.tombstones.push(pb::SourceIdentityTombstone {
            registration_id: registration.id,
            identity: Some(pb::source_identity_tombstone::Identity::RunId(req.run_id)),
            request_digest,
            branch: ctx.branch,
        });
        doc.state.runs.push(StoredRun {
            run: Some(run.clone()),
            payloads: req.native_payloads,
            values,
            transitions: 1,
        });
        if tokio::time::Instant::now() >= deadline {
            return Err(Status::deadline_exceeded(
                "source attempt deadline exceeded",
            ));
        }
        save(service, &mut doc).await?;
        if tokio::time::Instant::now() >= deadline {
            return Err(Status::deadline_exceeded(
                "source attempt deadline exceeded",
            ));
        }
        Ok(Response::new(pb::PreviewSourceRunResponse {
            run: Some(summary(&run)),
        }))
    })
    .await
    .map_err(|_| Status::deadline_exceeded("source attempt deadline exceeded"))?
}
async fn authorize_run(
    service: &EventModelServiceImpl,
    ctx: &Context,
    run: &pb::ReconciliationRun,
) -> Result<(), Status> {
    if run.branch != ctx.branch {
        return Err(missing());
    }
    if let Some(plan) = &run.plan {
        for guard in &plan.native_guards {
            let id = guard
                .owner
                .as_ref()
                .and_then(|o| o.id.as_ref())
                .ok_or_else(invalid)?;
            service.require_visible(&ctx.vis, &id.namespace).await?;
        }
    }
    Ok(())
}
async fn authorize_apply(
    service: &EventModelServiceImpl,
    ctx: &Context,
    registration: &pb::SourceRegistration,
    run: &pb::ReconciliationRun,
) -> Result<(), Status> {
    authorize_run(service, ctx, run).await?;
    ctx.authorize(service, registration, true).await?;
    let principal = ctx
        .principal
        .as_ref()
        .map_or("anonymous", |p| p.name.as_ref());
    if principal != run.principal {
        return Err(Status::permission_denied(
            "source preview belongs to another principal",
        ));
    }
    let mut targets = WriteTargets::none();
    if let Some(plan) = &run.plan {
        for operation in &plan.operations {
            let source_guard = match operation.operation.as_ref() {
                Some(pb::source_native_operation::Operation::Put(p)) => p.guard.as_ref(),
                Some(pb::source_native_operation::Operation::Delete(d)) => d.guard.as_ref(),
                None => None,
            }
            .ok_or_else(invalid)?;
            let id = source_guard
                .owner
                .as_ref()
                .and_then(|o| o.id.as_ref())
                .ok_or_else(invalid)?;
            if !registration.destination_namespaces.contains(&id.namespace) {
                return Err(Status::permission_denied(
                    "source destination no longer selected",
                ));
            }
            targets.insert(&id.namespace);
        }
    }
    service
        .authorize_write(ctx.principal.as_ref(), ctx.branch.as_deref(), &targets)
        .await
}
fn transition(state: &mut SourceState, index: usize) -> Result<(), Status> {
    let stored = &mut state.runs[index];
    let max = stored
        .run
        .as_ref()
        .and_then(|r| r.budgets.as_ref())
        .ok_or_else(invalid)?
        .control_transitions;
    stored.transitions = stored
        .transitions
        .checked_add(1)
        .filter(|n| *n <= max)
        .ok_or_else(|| Status::resource_exhausted("source control transition budget exhausted"))?;
    Ok(())
}
async fn settle(
    service: &EventModelServiceImpl,
    doc: &mut Document,
    index: usize,
    outcome: SourcePublicationOutcome,
) -> Result<pb::ReconciliationRun, Status> {
    let registration = registration(&doc.state)?.clone();
    let stored = doc.state.runs[index].clone();
    let mut run = stored.run.ok_or_else(invalid)?;
    let settled = now();
    match outcome {
        SourcePublicationOutcome::NotCommitted => {
            run.status = if run.cancellation
                == pb::SourceCancellationDisposition::RequestedPendingRecovery as i32
            {
                pb::SourceRunStatus::Cancelled as i32
            } else {
                pb::SourceRunStatus::Rejected as i32
            };
            run.recovery_required = false;
        }
        SourcePublicationOutcome::Committed(marker) => {
            if marker.run_id != run.id
                || marker.plan_digest != run.preview_digest
                || marker.branch != run.branch
            {
                return Err(conflict());
            }
            service.mirror_source_current(&marker, &run.principal).await;
            for value in &marker.values {
                let mapping = doc
                    .state
                    .mappings
                    .iter_mut()
                    .find(|m| m.id == value.mapping_id)
                    .ok_or_else(conflict)?;
                if value.deleted {
                    mapping.detached = true;
                    mapping.revision = revision();
                    mapping.last_accepted_run_id.clone_from(&run.id);
                    continue;
                }
                let source = stored
                    .payloads
                    .iter()
                    .find(|p| p.source == value.source)
                    .and_then(|p| p.entity.as_ref())
                    .ok_or_else(conflict)?;
                let native = value.entity.as_ref().ok_or_else(conflict)?;
                let ownership = mapping.ownership.as_ref().ok_or_else(invalid)?;
                let units = bounded(core::validate_ownership(native, ownership))?;
                let drift = run.results.iter().any(|r| {
                    r.mapping_id == mapping.id
                        && r.disposition == pb::SourceResultDisposition::LocalDrift as i32
                });
                let mut base = mapping.base.clone().unwrap_or(pb::SourceMergeBase {
                    source_value: Some(source.clone()),
                    native_value: Some(native.clone()),
                    ..Default::default()
                });
                let old_source = base.source_value.clone().ok_or_else(conflict)?;
                let old_native = base.native_value.clone().ok_or_else(conflict)?;
                base.source_value = Some(source.clone());
                base.native_value = Some(native.clone());
                for unit in units {
                    if run.results.iter().any(|r| {
                        r.mapping_id == mapping.id
                            && r.field == field(unit).map(|f| f as i32)
                            && r.disposition == pb::SourceResultDisposition::LocalDrift as i32
                    }) {
                        let source_unit = bounded(core::extract_unit(&old_source, unit))?;
                        let native_unit = bounded(core::extract_unit(&old_native, unit))?;
                        bounded(core::replace_unit(
                            base.source_value.as_mut().ok_or_else(invalid)?,
                            unit,
                            &source_unit,
                        ))?;
                        bounded(core::replace_unit(
                            base.native_value.as_mut().ok_or_else(invalid)?,
                            unit,
                            &native_unit,
                        ))?;
                    }
                }
                base.revision = revision();
                base.snapshot_id.clone_from(&run.snapshot_id);
                base.source_version.clone_from(
                    &doc.state
                        .snapshots
                        .iter()
                        .find(|s| s.id == run.snapshot_id)
                        .ok_or_else(conflict)?
                        .source_version,
                );
                base.observed.clone_from(&value.observed);
                base.branch.clone_from(&run.branch);
                base.ownership_policy_revision
                    .clone_from(&registration.ownership_policy_revision);
                base.run_id.clone_from(&run.id);
                if !drift {
                    base.accepted_at_unix_ms = settled;
                    base.expires_at_unix_ms = settled
                        + registration
                            .retention
                            .as_ref()
                            .ok_or_else(invalid)?
                            .base_seconds
                            .cast_signed()
                            * 1000;
                }
                mapping.base = Some(base);
                mapping.observed.clone_from(&value.observed);
                mapping.last_accepted_run_id.clone_from(&run.id);
                mapping.revision = revision();
            }
            for result in &run.results {
                if result.disposition == pb::SourceResultDisposition::Detach as i32 {
                    let mapping = doc
                        .state
                        .mappings
                        .iter_mut()
                        .find(|m| m.id == result.mapping_id)
                        .ok_or_else(conflict)?;
                    mapping.detached = true;
                    mapping.revision = revision();
                    mapping.last_accepted_run_id.clone_from(&run.id);
                }
            }
            run.marker = Some(marker);
            run.status = pb::SourceRunStatus::Applied as i32;
            run.recovery_required = false;
            if run.cancellation
                == pb::SourceCancellationDisposition::RequestedPendingRecovery as i32
            {
                run.cancellation = pb::SourceCancellationDisposition::AlreadyCommitted as i32;
            }
        }
    }
    if run.status == pb::SourceRunStatus::Applied as i32 {
        let snapshot = doc
            .state
            .snapshots
            .iter_mut()
            .find(|s| s.id == run.snapshot_id)
            .ok_or_else(conflict)?;
        if snapshot.accepted_at_unix_ms.is_none() {
            snapshot.accepted_at_unix_ms = Some(settled);
            snapshot.expires_at_unix_ms = settled
                + registration
                    .retention
                    .as_ref()
                    .ok_or_else(invalid)?
                    .accepted_snapshot_seconds
                    .cast_signed()
                    * 1000;
        }
    }
    run.revision = revision();
    run.settled_at_unix_ms = Some(settled);
    run.expires_at_unix_ms = settled
        + registration
            .retention
            .as_ref()
            .ok_or_else(invalid)?
            .settled_outcome_seconds
            .cast_signed()
            * 1000;
    transition(&mut doc.state, index)?;
    doc.state.runs[index].run = Some(run.clone());
    save(service, doc).await?;
    service.invalidate_snapshot().await;
    if run.status == pb::SourceRunStatus::Applied as i32 && !run.overlay_only {
        service
            .search_ready
            .store(false, std::sync::atomic::Ordering::Release);
    }
    Ok(run)
}
fn live_publication_bases(state: &SourceState, index: usize) -> Result<(), Status> {
    let stored = &state.runs[index];
    let plan = stored
        .run
        .as_ref()
        .and_then(|run| run.plan.as_ref())
        .ok_or_else(conflict)?;
    for value in &stored.values {
        let guard = plan
            .mapping_guards
            .iter()
            .find(|guard| guard.mapping_id == value.mapping_id)
            .ok_or_else(conflict)?;
        if guard.base_revision.is_empty() {
            continue;
        }
        let base = state
            .mappings
            .iter()
            .find(|mapping| mapping.id == value.mapping_id)
            .and_then(|mapping| mapping.base.as_ref())
            .ok_or_else(conflict)?;
        if base.expires_at_unix_ms <= now()
            || base.source_value.is_none()
            || base.native_value.is_none()
        {
            return Err(Status::failed_precondition(
                "source base expired; explicit re-adoption is required",
            ));
        }
    }
    Ok(())
}
pub(super) async fn apply_source_run(
    service: &EventModelServiceImpl,
    request: Request<pb::ApplySourceRunRequest>,
) -> Result<Response<pb::ApplySourceRunResponse>, Status> {
    let started = tokio::time::Instant::now();
    let ceiling = std::time::Duration::from_millis(core::profile_budgets().phase_timeout_ms);
    tokio::time::timeout_at(
        started + ceiling,
        apply_source_run_attempt(service, request, started),
    )
    .await
    .map_err(|_| Status::deadline_exceeded("source attempt deadline exceeded"))?
}
async fn apply_source_run_attempt(
    service: &EventModelServiceImpl,
    request: Request<pb::ApplySourceRunRequest>,
    started: tokio::time::Instant,
) -> Result<Response<pb::ApplySourceRunResponse>, Status> {
    use pb::event_model_service_server::EventModelService;
    let ctx = Context::new(&request, crate::auth::Role::Writer)?;
    let req = request.into_inner();
    let lock = service.lock_mutations().await;
    let mut doc = load(service, &ctx, &req.registration_id, true).await?;
    let index = run_index(&doc.state, &req.run_id)?;
    let mut run = doc.state.runs[index].run.clone().ok_or_else(invalid)?;
    let deadline = started
        + std::time::Duration::from_millis(
            run.budgets.as_ref().ok_or_else(invalid)?.phase_timeout_ms,
        );
    tokio::time::timeout_at(deadline,async move {
    authorize_apply(service, &ctx, registration(&doc.state)?, &run).await?;
    if req.preview_digest != run.preview_digest {
        return Err(conflict());
    }
    if run.status == pb::SourceRunStatus::Applied as i32
        || run.status == pb::SourceRunStatus::Rejected as i32
        || run.status == pb::SourceRunStatus::Cancelled as i32
    {
        return Ok(Response::new(pb::ApplySourceRunResponse {
            run: Some(summary(&run)),
        }));
    }
    guard(&run.revision, &req.if_match)?;
    if run.status != pb::SourceRunStatus::Previewed as i32
        || run.expires_at_unix_ms <= now()
        || run.recovery_required
    {
        return Err(conflict());
    }
    if run
        .results
        .iter()
        .any(|r| r.disposition == pb::SourceResultDisposition::Conflict as i32)
    {
        return Err(Status::failed_precondition(
            "source preview has unresolved conflicts",
        ));
    }
    let registration = registration(&doc.state)?.clone();
    if doc
        .state
        .snapshots
        .iter()
        .find(|s| s.id == run.snapshot_id)
        .is_none_or(|s| !s.bytes_available || s.expires_at_unix_ms <= now())
    {
        return Err(Status::failed_precondition(
            "source snapshot retention expired",
        ));
    }
    let plan = run.plan.as_ref().ok_or_else(conflict)?.clone();
    if plan.registration_revision != registration.revision
        || run.configuration_digest != digest(&registration)
    {
        return Err(conflict());
    }
    for guard in &plan.mapping_guards {
        let mapping = doc
            .state
            .mappings
            .iter()
            .find(|m| m.id == guard.mapping_id)
            .ok_or_else(conflict)?;
        if mapping.detached
            || mapping.revision != guard.revision
            || mapping.base.as_ref().map_or("", |b| b.revision.as_str()) != guard.base_revision
        {
            return Err(conflict());
        }
        ownership_available(service, mapping, &registration.id).await?;
    }
    if run.overlay_only {
        run.status = pb::SourceRunStatus::Applied as i32;
        run.settled_at_unix_ms = Some(now());
        run.revision = revision();
        run.expires_at_unix_ms = now()
            + registration
                .retention
                .as_ref()
                .ok_or_else(invalid)?
                .settled_outcome_seconds
                .cast_signed()
                * 1000;
        transition(&mut doc.state, index)?;
        doc.state.runs[index].run = Some(run.clone());
        save(service, &mut doc).await?;
        return Ok(Response::new(pb::ApplySourceRunResponse {
            run: Some(summary(&run)),
        }));
    }
    live_publication_bases(&doc.state, index)?;
    for operation in &plan.operations {
        let Some(pb::source_native_operation::Operation::Put(put)) = &operation.operation else {
            continue;
        };
        let previous = doc.state.runs[index].values.iter()
            .find(|value| value.mapping_id == operation.mapping_id)
            .ok_or_else(conflict)?;
        bounded(core::validate_native_schema_variants(
            previous.entity.as_ref(), put.entity.as_ref().ok_or_else(conflict)?,
        ))?;
    }
    let mut publication = SourcePublication {
        registration: doc.key.clone(),
        control_revision: doc.revision,
        run_id: run.id.clone(),
        plan_digest: run.preview_digest.clone(),
        plan: plan.clone(),
        values: doc.state.runs[index].values.clone(),
        storage_limit: registration
            .retention
            .as_ref()
            .ok_or_else(invalid)?
            .registration_bytes,
        finalization_reserve_bytes: doc.state.encoded_len() as u64
            + doc.state.runs[index]
                .payloads
                .iter()
                .map(|p| p.encoded_len() as u64 + 65536)
                .sum::<u64>()
                * 4,
    };
    service
        .store
        .source_prepare(&publication)
        .await
        .map_err(store_err)?;
    live_publication_bases(&doc.state, index)?;
    run.status = pb::SourceRunStatus::Applying as i32;
    run.recovery_required = true;
    run.revision = revision();
    transition(&mut doc.state, index)?;
    doc.state.runs[index].run = Some(run.clone());
    save(service, &mut doc).await?;
    publication.control_revision = doc.revision;
    let execution = SourceExecution {
        publication,
        outcome: Arc::new(Mutex::new(None)),
    };
    drop(lock);
    let mut native = ctx.request(batch_request(&plan, false)?);
    native.extensions_mut().insert(execution.clone());
    native.extensions_mut().insert(SourceBusinessBypass);
    let result = tokio::time::timeout_at(deadline, service.batch_mutate(native)).await;
    let _lock = service.lock_mutations().await;
    let mut doc = load(service, &ctx, &req.registration_id, true).await?;
    let index = run_index(&doc.state, &req.run_id)?;
    let current = doc.state.runs[index].run.as_ref().ok_or_else(invalid)?;
    if !current.recovery_required || current.status != pb::SourceRunStatus::Applying as i32 {
        return Ok(Response::new(pb::ApplySourceRunResponse {
            run: Some(summary(current)),
        }));
    }
    let outcome = execution
        .outcome
        .lock()
        .map_err(|_| Status::internal("source outcome lock unavailable"))?
        .clone();
    if let Some(outcome) = outcome {
        let run = settle(service, &mut doc, index, outcome).await?;
        return Ok(Response::new(pb::ApplySourceRunResponse {
            run: Some(summary(&run)),
        }));
    }
    let native_refused = matches!(&result,Ok(Ok(response)) if response.get_ref().status!=pb::batch_mutate_response::Status::Applied as i32);
    if let Ok(Ok(outcome)) = tokio::time::timeout_at(
        deadline,
        service
            .store
            .source_recover(&doc.key, &run.id, &run.preview_digest),
    )
    .await
    {
        let settled = settle(service, &mut doc, index, outcome).await?;
        if settled.status == pb::SourceRunStatus::Applied as i32 {
            return Ok(Response::new(pb::ApplySourceRunResponse {
                run: Some(summary(&settled)),
            }));
        }
        return match result {
            Ok(Err(status)) => Err(status),
            _ => Err(Status::failed_precondition(
                "native admission refused source publication",
            )),
        };
    }
    let current = doc.state.runs[index].run.as_mut().ok_or_else(invalid)?;
    current.status = pb::SourceRunStatus::OutcomeUnknown as i32;
    current.recovery_required = true;
    current.revision = revision();
    save(service, &mut doc).await?;
    if native_refused {
        return Err(Status::failed_precondition(
            "native admission refused source publication; recovery is required",
        ));
    }
    match result {
        Ok(Err(status)) => Err(status),
        Err(_) => Err(Status::deadline_exceeded(
            "source publication outcome requires recovery",
        )),
        _ => Err(Status::unavailable(
            "source publication outcome requires recovery",
        )),
    }
    }).await.map_err(|_|Status::deadline_exceeded("source attempt deadline exceeded"))?
}
pub(super) async fn recover_source_run(
    service: &EventModelServiceImpl,
    request: Request<pb::RecoverSourceRunRequest>,
) -> Result<Response<pb::RecoverSourceRunResponse>, Status> {
    let started = tokio::time::Instant::now();
    let ceiling = std::time::Duration::from_millis(core::profile_budgets().phase_timeout_ms);
    tokio::time::timeout_at(
        started + ceiling,
        recover_source_run_attempt(service, request, started),
    )
    .await
    .map_err(|_| Status::deadline_exceeded("source attempt deadline exceeded"))?
}
async fn recover_source_run_attempt(
    service: &EventModelServiceImpl,
    request: Request<pb::RecoverSourceRunRequest>,
    started: tokio::time::Instant,
) -> Result<Response<pb::RecoverSourceRunResponse>, Status> {
    let ctx = Context::new(&request, crate::auth::Role::Writer)?;
    let req = request.into_inner();
    let _lock = service.lock_mutations().await;
    let mut doc = load(service, &ctx, &req.registration_id, true).await?;
    let index = run_index(&doc.state, &req.run_id)?;
    let run = doc.state.runs[index]
        .run
        .as_ref()
        .ok_or_else(invalid)?
        .clone();
    let deadline = started
        + std::time::Duration::from_millis(
            run.budgets.as_ref().ok_or_else(invalid)?.phase_timeout_ms,
        );
    tokio::time::timeout_at(deadline, async move {
        authorize_apply(service, &ctx, registration(&doc.state)?, &run).await?;
        if !run.recovery_required {
            return Ok(Response::new(pb::RecoverSourceRunResponse {
                run: Some(summary(&run)),
            }));
        }
        guard(&run.revision, &req.if_match)?;
        transition(&mut doc.state, index)?;
        save(service, &mut doc).await?;
        let outcome = tokio::time::timeout_at(
            deadline,
            service
                .store
                .source_recover(&doc.key, &run.id, &run.preview_digest),
        )
        .await
        .map_err(|_| Status::deadline_exceeded("source recovery remains unsettled"))?
        .map_err(store_err)?;
        let run = settle(service, &mut doc, index, outcome).await?;
        Ok(Response::new(pb::RecoverSourceRunResponse {
            run: Some(summary(&run)),
        }))
    })
    .await
    .map_err(|_| Status::deadline_exceeded("source attempt deadline exceeded"))?
}
async fn settle_unpublished_intent(
    store: &dyn trogon_atlas_store::Store,
    key: &SourceDocumentKey,
    control_revision: u64,
    run: &pb::ReconciliationRun,
    storage_limit: u64,
) -> Result<(), StoreError> {
    let publication = SourcePublication {
        registration: key.clone(),
        control_revision,
        run_id: run.id.clone(),
        plan_digest: run.preview_digest.clone(),
        plan: run
            .plan
            .clone()
            .ok_or_else(|| StoreError::Unavailable("source preparation plan unavailable".into()))?,
        values: Vec::new(),
        storage_limit,
        finalization_reserve_bytes: 0,
    };
    store.source_prepare(&publication).await?;
    match store
        .source_recover(key, &run.id, &run.preview_digest)
        .await?
    {
        SourcePublicationOutcome::NotCommitted => Ok(()),
        SourcePublicationOutcome::Committed(_) => Err(StoreError::Unavailable(
            "source publication requires committed outcome recovery".into(),
        )),
    }
}
pub(super) async fn cancel_source_run(
    service: &EventModelServiceImpl,
    request: Request<pb::CancelSourceRunRequest>,
) -> Result<Response<pb::CancelSourceRunResponse>, Status> {
    let ctx = Context::new(&request, crate::auth::Role::Writer)?;
    let req = request.into_inner();
    let _lock = service.lock_mutations().await;
    let mut doc = load(service, &ctx, &req.registration_id, true).await?;
    let index = run_index(&doc.state, &req.run_id)?;
    let mut run = doc.state.runs[index]
        .run
        .as_ref()
        .ok_or_else(invalid)?
        .clone();
    authorize_apply(service, &ctx, registration(&doc.state)?, &run).await?;
    guard(&run.revision, &req.if_match)?;
    match pb::SourceRunStatus::try_from(run.status).map_err(|_| invalid())? {
        pb::SourceRunStatus::Prepared | pb::SourceRunStatus::Previewed => {
            let timeout = std::time::Duration::from_millis(
                run.budgets.as_ref().ok_or_else(invalid)?.phase_timeout_ms,
            );
            tokio::time::timeout(
                timeout,
                settle_unpublished_intent(
                    &*service.store,
                    &doc.key,
                    doc.revision,
                    &run,
                    registration(&doc.state)?
                        .retention
                        .as_ref()
                        .ok_or_else(invalid)?
                        .registration_bytes,
                ),
            )
            .await
            .map_err(|_| Status::deadline_exceeded("source cancellation requires recovery"))?
            .map_err(store_err)?;

            run.status = pb::SourceRunStatus::Cancelled as i32;
            run.cancellation = pb::SourceCancellationDisposition::PreventedPublication as i32;
            run.recovery_required = false;
            run.settled_at_unix_ms = Some(now());
            run.expires_at_unix_ms = now()
                + registration(&doc.state)?
                    .retention
                    .as_ref()
                    .ok_or_else(invalid)?
                    .settled_outcome_seconds
                    .cast_signed()
                    * 1000;
        }
        pb::SourceRunStatus::Applying | pb::SourceRunStatus::OutcomeUnknown => {
            run.cancellation = pb::SourceCancellationDisposition::RequestedPendingRecovery as i32;
            run.recovery_required = true;
        }
        pb::SourceRunStatus::Applied => {
            run.cancellation = pb::SourceCancellationDisposition::AlreadyCommitted as i32;
        }
        _ => {}
    }
    run.revision = revision();
    transition(&mut doc.state, index)?;
    doc.state.runs[index].run = Some(run.clone());
    save(service, &mut doc).await?;
    Ok(Response::new(pb::CancelSourceRunResponse {
        run: Some(summary(&run)),
    }))
}
pub(super) async fn get_source_run(
    service: &EventModelServiceImpl,
    request: Request<pb::GetSourceRunRequest>,
) -> Result<Response<pb::GetSourceRunResponse>, Status> {
    let ctx = Context::new(&request, crate::auth::Role::Reader)?;
    let req = request.into_inner();
    let doc = load(service, &ctx, &req.registration_id, false).await?;
    let run = get_run(&doc.state, &req.run_id)?
        .run
        .as_ref()
        .ok_or_else(missing)?;
    authorize_run(service, &ctx, run).await?;
    Ok(Response::new(pb::GetSourceRunResponse {
        run: Some(summary(run)),
    }))
}
pub(super) async fn list_source_runs(
    service: &EventModelServiceImpl,
    request: Request<pb::ListSourceRunsRequest>,
) -> Result<Response<pb::ListSourceRunsResponse>, Status> {
    let ctx = Context::new(&request, crate::auth::Role::Reader)?;
    let req = request.into_inner();
    let doc = load(service, &ctx, &req.registration_id, false).await?;
    let budgets = registration(&doc.state)?
        .budgets
        .as_ref()
        .ok_or_else(invalid)?;
    let mut items = Vec::new();
    for run in doc.state.runs.iter().filter_map(|r| r.run.as_ref()) {
        match authorize_run(service, &ctx, run).await {
            Ok(()) => items.push(summary(run)),
            Err(status) if status.code() == tonic::Code::NotFound => {}
            Err(status) => return Err(status),
        }
    }
    let (runs, next_page_token) = page(
        &items,
        page_start(&req.page_token)?,
        page_limit(req.page_size, budgets.page_records)?,
        budgets.page_bytes,
    )?;
    Ok(Response::new(pb::ListSourceRunsResponse {
        runs,
        next_page_token,
    }))
}
fn run_cursor(token: &str, run: &pb::ReconciliationRun) -> Result<usize, Status> {
    if token.is_empty() {
        return Ok(0);
    }
    let (rev, index) = token.split_once(':').ok_or_else(invalid)?;
    guard(&run.revision, rev)?;
    page_start(index)
}
fn next_run_cursor(cursor: String, run: &pb::ReconciliationRun) -> String {
    if cursor.is_empty() {
        cursor
    } else {
        format!("{}:{cursor}", run.revision)
    }
}
pub(super) async fn list_source_run_operations(
    service: &EventModelServiceImpl,
    request: Request<pb::ListSourceRunOperationsRequest>,
) -> Result<Response<pb::ListSourceRunOperationsResponse>, Status> {
    let ctx = Context::new(&request, crate::auth::Role::Reader)?;
    let req = request.into_inner();
    let doc = load(service, &ctx, &req.registration_id, false).await?;
    let run = get_run(&doc.state, &req.run_id)?
        .run
        .as_ref()
        .ok_or_else(missing)?;
    authorize_run(service, &ctx, run).await?;
    if run.expires_at_unix_ms <= now() && !run.recovery_required {
        return Err(Status::failed_precondition("source plan retention expired"));
    }
    let plan = run
        .plan
        .as_ref()
        .ok_or_else(|| Status::failed_precondition("source plan retention expired"))?;
    let budgets = run.budgets.as_ref().ok_or_else(invalid)?;
    let mut items = Vec::new();
    items.extend(
        plan.operations
            .iter()
            .cloned()
            .map(|v| pb::SourceRunOperationEntry {
                entry: Some(pb::source_run_operation_entry::Entry::Operation(v)),
            }),
    );
    items.extend(
        plan.native_guards
            .iter()
            .cloned()
            .map(|v| pb::SourceRunOperationEntry {
                entry: Some(pb::source_run_operation_entry::Entry::NativeGuard(v)),
            }),
    );
    items.extend(
        plan.mapping_guards
            .iter()
            .cloned()
            .map(|v| pb::SourceRunOperationEntry {
                entry: Some(pb::source_run_operation_entry::Entry::MappingGuard(v)),
            }),
    );
    let (entries, cursor) = page(
        &items,
        run_cursor(&req.page_token, run)?,
        page_limit(req.page_size, budgets.page_records)?,
        budgets.page_bytes.saturating_sub(1024),
    )?;
    Ok(Response::new(pb::ListSourceRunOperationsResponse {
        entries,
        next_page_token: next_run_cursor(cursor, run),
        run_revision: run.revision.clone(),
        preview_digest: run.preview_digest.clone(),
        incomplete: run.incomplete,
    }))
}
pub(super) async fn list_source_run_results(
    service: &EventModelServiceImpl,
    request: Request<pb::ListSourceRunResultsRequest>,
) -> Result<Response<pb::ListSourceRunResultsResponse>, Status> {
    let ctx = Context::new(&request, crate::auth::Role::Reader)?;
    let req = request.into_inner();
    let doc = load(service, &ctx, &req.registration_id, false).await?;
    let run = get_run(&doc.state, &req.run_id)?
        .run
        .as_ref()
        .ok_or_else(missing)?;
    authorize_run(service, &ctx, run).await?;
    let mut fidelity = run
        .fidelity
        .clone()
        .ok_or_else(|| Status::failed_precondition("source result retention expired"))?;
    let records = std::mem::take(&mut fidelity.records);
    let owners = std::mem::take(&mut fidelity.checked_owners);
    let budgets = run.budgets.as_ref().ok_or_else(invalid)?;
    let start = run_cursor(&req.page_token, run)?;
    let count = run.results.len() + records.len() + owners.len();
    if start > count {
        return Err(invalid());
    }
    let mut response = pb::ListSourceRunResultsResponse {
        results: Vec::new(),
        fidelity: Some(fidelity),
        next_page_token: String::new(),
        incomplete: run.incomplete,
        run_revision: run.revision.clone(),
        preview_digest: run.preview_digest.clone(),
    };
    let limit = page_limit(req.page_size, budgets.page_records)?;
    let mut end = start;
    while end < count && end - start < limit {
        let mut candidate = response.clone();
        if end < run.results.len() {
            candidate.results.push(run.results[end].clone());
        } else if end < run.results.len() + records.len() {
            candidate
                .fidelity
                .as_mut()
                .ok_or_else(invalid)?
                .records
                .push(records[end - run.results.len()].clone());
        } else {
            candidate
                .fidelity
                .as_mut()
                .ok_or_else(invalid)?
                .checked_owners
                .push(owners[end - run.results.len() - records.len()].clone());
        }
        if candidate.encoded_len() as u64 + 256 > budgets.page_bytes {
            if end == start {
                return Err(Status::resource_exhausted(
                    "source result exceeds page budget",
                ));
            }
            break;
        }
        response = candidate;
        end += 1;
    }
    if end < count {
        response.next_page_token = next_run_cursor(end.to_string(), run);
    }
    Ok(Response::new(response))
}

pub(super) struct RetentionTask(tokio::task::JoinHandle<()>);
impl Drop for RetentionTask {
    fn drop(&mut self) {
        self.0.abort();
    }
}
pub(super) fn spawn_retention_sweep(
    store: Arc<dyn trogon_atlas_store::Store>,
) -> Option<RetentionTask> {
    let runtime = tokio::runtime::Handle::try_current().ok()?;
    Some(RetentionTask(runtime.spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
        let mut cursor = None;
        let mut pending = std::collections::VecDeque::<SourceDocumentKey>::new();
        let mut end_of_pass = false;
        loop {
            interval.tick().await;
            if !store.is_writer() {
                continue;
            }
            let mut active_key = None;
            let attempt = async {
                loop {
                    if let Some(key) = pending.pop_front() {
                        // Cancellation must not pin later registrations behind this key.
                        active_key = Some(key.as_str().to_owned());
                        if let Err(error) = sweep_registration(&*store, &key).await {
                            tracing::warn!(registration = key.as_str(), %error, "source retention sweep skipped a registration");
                        }
                        active_key = None;
                    } else if end_of_pass {
                        end_of_pass = false;
                        break;
                    } else {
                        let page = store.source_list(cursor.as_deref(), 100).await?;
                        cursor = page.next_cursor;
                        pending.extend(page.keys);
                        end_of_pass = cursor.is_none();
                    }
                }
                Ok::<(), StoreError>(())
            };
            match tokio::time::timeout(std::time::Duration::from_secs(60), attempt).await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => tracing::warn!(%error, "source retention sweep could not list registrations"),
                Err(_) => tracing::warn!(registration = ?active_key, "source retention sweep timed out; remaining registrations resume next tick"),
            }
        }
    })))
}

async fn sweep_registration(
    store: &dyn trogon_atlas_store::Store,
    key: &SourceDocumentKey,
) -> Result<(), StoreError> {
    let Some(document) = store.source_get(key).await? else {
        return Ok(());
    };
    let mut state = SourceState::decode(document.bytes.as_slice())
        .map_err(|_| StoreError::Unavailable("source retention state unavailable".into()))?;
    if state.deleted {
        return Ok(());
    }
    let retention = state
        .registration
        .as_ref()
        .and_then(|r| r.retention)
        .ok_or_else(|| {
            StoreError::Unavailable("source retention configuration unavailable".into())
        })?;
    for stored in &mut state.runs {
        let Some(run) = &mut stored.run else {
            continue;
        };
        if !run.recovery_required
            && run.settled_at_unix_ms.is_none()
            && run.expires_at_unix_ms <= now()
            && matches!(
                pb::SourceRunStatus::try_from(run.status),
                Ok(pb::SourceRunStatus::Previewed
                    | pb::SourceRunStatus::Prepared
                    | pb::SourceRunStatus::Rejected)
            )
        {
            settle_unpublished_intent(
                store,
                key,
                document.revision,
                run,
                retention.registration_bytes,
            )
            .await?;
            run.status = pb::SourceRunStatus::Rejected as i32;
            run.settled_at_unix_ms = Some(run.expires_at_unix_ms);
            run.revision = revision();
            stored.payloads.clear();
            stored.values.clear();
        }
    }
    for run in state.runs.iter().filter_map(|r| r.run.as_ref()) {
        if !run.recovery_required
            && run.settled_at_unix_ms.is_some_and(|at| {
                at.saturating_add(retention.marker_seconds.cast_signed() * 1000) <= now()
            })
        {
            store
                .source_expire_transaction(key, &run.id, &run.preview_digest)
                .await?;
        }
    }
    expire(&mut state);
    let bytes = state.encode_to_vec();
    if bytes != document.bytes {
        store
            .source_compare_and_swap(
                key,
                Some(document.revision),
                &bytes,
                retention.registration_bytes,
            )
            .await?;
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod native_authoring_tests {
    use super::*;

    fn id(slug: &str) -> pb::Id {
        pb::Id {
            namespace: "groups".into(),
            slug: slug.into(),
            version: 1,
        }
    }

    fn group(members: usize, relationships: usize, actors: usize, title: &str) -> pb::Entity {
        let member = pb::SystemGroupMembership {
            member: Some(pb::SystemGroupMember {
                target: Some(pb::system_group_member::Target::Component(
                    pb::ComponentRef {
                        id: Some(id("worker")),
                    },
                )),
            }),
            ..Default::default()
        };
        let relationship = pb::SystemGroupRelationship {
            target: Some(pb::SystemGroupRef {
                id: Some(id("other")),
            }),
            ..Default::default()
        };
        let actor = pb::SystemActorLink {
            target: Some(pb::SystemActorTarget {
                target: Some(pb::system_actor_target::Target::Persona(pb::PersonaRef {
                    id: Some(id("operator")),
                })),
            }),
            direction: pb::SystemActorDirection::Inbound as i32,
            ..Default::default()
        };
        pb::Entity {
            kind: Some(pb::entity::Kind::SystemGroup(pb::SystemGroup {
                id: Some(id("platform")),
                title: title.into(),
                members: vec![member; members],
                relationships: vec![relationship; relationships],
                actors: vec![actor; actors],
                ..Default::default()
            })),
            ..Default::default()
        }
    }

    fn domain(members: usize) -> pb::Entity {
        let member = pb::DomainMembership {
            member: Some(pb::DomainMember {
                target: Some(pb::domain_member::Target::Component(pb::ComponentRef {
                    id: Some(id("worker")),
                })),
            }),
            ..Default::default()
        };
        pb::Entity {
            kind: Some(pb::entity::Kind::Domain(pb::Domain {
                id: Some(id("commerce")),
                members: vec![member; members],
                ..Default::default()
            })),
            ..Default::default()
        }
    }

    fn policies() -> Vec<pb::SourceFieldOwnership> {
        let policy = |value| pb::SourceFieldOwnership {
            policy: Some(value),
        };
        vec![
            policy(pb::source_field_ownership::Policy::None(
                pb::SourceNoOwnership {},
            )),
            policy(pb::source_field_ownership::Policy::Fields(
                pb::SourceOwnedFields {
                    fields: vec![pb::SourceOwnedField::Title as i32],
                },
            )),
            policy(pb::source_field_ownership::Policy::WholeOwner(
                pb::SourceWholeOwnerOwnership {},
            )),
        ]
    }

    fn whole_owner() -> pb::SourceFieldOwnership {
        pb::SourceFieldOwnership {
            policy: Some(pb::source_field_ownership::Policy::WholeOwner(
                pb::SourceWholeOwnerOwnership {},
            )),
        }
    }

    fn field_owner() -> pb::SourceFieldOwnership {
        pb::SourceFieldOwnership {
            policy: Some(pb::source_field_ownership::Policy::Fields(
                pb::SourceOwnedFields {
                    fields: vec![pb::SourceOwnedField::Title as i32],
                },
            )),
        }
    }

    #[test]
    fn initial_creation_with_native_curation_is_refused_under_every_policy() {
        for ownership in policies() {
            for incoming in [
                group(1, 0, 0, "t"),
                group(0, 1, 0, "t"),
                group(0, 0, 1, "t"),
                domain(1),
            ] {
                let refused = native_relationships_only(None, Some(&incoming), &ownership);
                assert_eq!(refused.unwrap_err().code(), tonic::Code::InvalidArgument);
            }
            assert!(
                native_relationships_only(None, Some(&group(0, 0, 0, "t")), &ownership).is_ok()
            );
            assert!(native_relationships_only(None, Some(&domain(0)), &ownership).is_ok());
        }
    }

    #[test]
    fn whole_owner_refuses_current_proposed_and_clearing_curation() {
        let ownership = whole_owner();
        let curated = group(2, 1, 1, "t");
        let empty = group(0, 0, 0, "t");
        for (current, incoming) in [
            (Some(&curated), Some(&curated)),
            (Some(&empty), Some(&curated)),
            (Some(&curated), Some(&empty)),
            (Some(&curated), None),
        ] {
            let refused = native_relationships_only(current, incoming, &ownership);
            assert_eq!(refused.unwrap_err().code(), tonic::Code::InvalidArgument);
        }
        assert_eq!(
            native_relationships_only(Some(&domain(1)), Some(&domain(0)), &ownership)
                .unwrap_err()
                .code(),
            tonic::Code::InvalidArgument
        );
        assert!(native_relationships_only(Some(&empty), Some(&empty), &ownership).is_ok());
    }

    #[test]
    fn unrelated_field_writes_preserve_local_curation_under_field_ownership() {
        let ownership = field_owner();
        let curated = group(2, 1, 1, "before");
        let retitled = group(2, 1, 1, "after");
        assert!(native_relationships_only(Some(&curated), Some(&retitled), &ownership).is_ok());
        assert!(native_relationships_only(Some(&domain(1)), Some(&domain(1)), &ownership).is_ok());
    }
}
