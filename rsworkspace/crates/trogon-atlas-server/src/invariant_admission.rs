use std::{
    collections::{BTreeSet, HashMap, HashSet},
    future::Future,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

use tonic::{Request, Response};

use super::{
    entity_id, entity_kind, pb, snapshot_digest, store_err, world_after, EventModelServiceImpl,
    ListFilter, Status, StoredEntity, Visibility,
};
use crate::{
    conv,
    invariants::{self, Worlds},
};

/// More event models than this in one scope and the scope's carriers are not
/// enumerated: every write in it is judged against a snapshot instead.
const MAX_REGISTRY_MODELS: usize = 5_000;

/// Write clocks and the per-scope carrier registry. Clocks only move
/// forward, even when a writer outside the mutation lock publishes its tick
/// after a later one, so a verdict whose clocks still match at apply time
/// was computed against the state the write lands in.
pub(super) struct InvariantState {
    budget: usize,
    tick: AtomicU64,
    baseline: AtomicU64,
    branches: std::sync::Mutex<HashMap<String, u64>>,
    carriers: AtomicU64,
    registry: tokio::sync::Mutex<HashMap<Option<String>, (u64, Registry)>>,
}

impl Default for InvariantState {
    fn default() -> Self {
        Self::with_budget(invariants::DEFAULT_BUDGET)
    }
}

impl InvariantState {
    pub(super) fn with_budget(budget: usize) -> Self {
        Self {
            budget,
            tick: AtomicU64::new(0),
            baseline: AtomicU64::new(0),
            branches: std::sync::Mutex::new(HashMap::new()),
            carriers: AtomicU64::new(0),
            registry: tokio::sync::Mutex::new(HashMap::new()),
        }
    }

    fn next_tick(&self) -> u64 {
        self.tick.fetch_add(1, Ordering::AcqRel) + 1
    }

    fn branch_clock(&self, branch: &str) -> u64 {
        self.branches
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(branch)
            .copied()
            .unwrap_or_default()
    }
}

#[derive(Clone)]
enum Registry {
    Carriers(Arc<Vec<StoredEntity>>),
    Unbounded,
}

impl Registry {
    fn declares(&self) -> bool {
        match self {
            Self::Carriers(carriers) => !carriers.is_empty(),
            Self::Unbounded => true,
        }
    }
}

/// Which world a write is judged in and which clocks its verdict pins.
#[derive(Clone, Copy)]
pub(super) enum InvariantScope<'a> {
    Baseline,
    Branch(&'a str),
    /// A branch landing on baseline: judged in baseline, pinned to both.
    Landing(&'a str),
}

impl<'a> InvariantScope<'a> {
    pub(super) fn of(branch: Option<&'a str>) -> Self {
        branch.map_or(Self::Baseline, Self::Branch)
    }

    fn world(self) -> Option<&'a str> {
        match self {
            Self::Baseline | Self::Landing(_) => None,
            Self::Branch(branch) => Some(branch),
        }
    }

    fn branch(self) -> Option<&'a str> {
        match self {
            Self::Baseline => None,
            Self::Branch(branch) | Self::Landing(branch) => Some(branch),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct Generations {
    baseline: u64,
    branch: Option<u64>,
    carriers: u64,
}

/// What a write learned about its invariants before taking the lock.
pub(super) enum InvariantTicket {
    /// The scope declared no invariant and the write adds none.
    Unguarded { carriers: u64 },
    Judged {
        generations: Generations,
        verdict: Result<Option<pb::InvariantCheck>, Status>,
    },
    /// The preview that would have judged the write failed for another
    /// reason; a write that gets past its own admission is refused as stale.
    Unjudged,
}

impl InvariantTicket {
    /// What a dry run reports: the verdict as computed, without the
    /// staleness check only an apply needs.
    pub(super) fn preview(&self) -> Result<Option<pb::InvariantCheck>, Status> {
        match self {
            Self::Unguarded { .. } => Ok(None),
            Self::Judged { verdict, .. } => verdict.clone(),
            Self::Unjudged => Err(conv::invariant_stale()),
        }
    }
}

/// One entity the write puts (`Some`) or removes (`None`).
pub(super) type Change = (pb::EntityKind, pb::Id, Option<pb::Entity>);

/// A copy of `req` carrying `message`, for running a route's own dry run
/// with the caller's identity, branch and extensions.
pub(super) fn preview_request<T, U>(req: &Request<T>, message: U) -> Request<U> {
    Request::from_parts(req.metadata().clone(), req.extensions().clone(), message)
}

pub(super) fn changes_of(
    keys: impl IntoIterator<Item = (pb::EntityKind, pb::Id)>,
    outcomes: &[trogon_atlas_store::store::MutationOutcome],
) -> Vec<Change> {
    use trogon_atlas_store::store::MutationOutcome;
    keys.into_iter()
        .zip(outcomes)
        .filter_map(|((kind, id), outcome)| match outcome {
            MutationOutcome::Wrote { entity, .. } => Some((kind, id, Some(entity.clone()))),
            MutationOutcome::Deleted => Some((kind, id, None)),
            MutationOutcome::Noop { .. } => None,
        })
        .collect()
}

impl EventModelServiceImpl {
    /// Moves the clocks a committed write invalidates. Called by every path
    /// that changes entities or branches, including branch lifecycle paths
    /// that run outside the mutation lock.
    pub(super) fn note_write(&self, branch: Option<&str>, touches_models: bool) {
        let tick = self.invariants.next_tick();
        self.publish_tick(branch, touches_models, tick);
    }

    fn publish_tick(&self, branch: Option<&str>, touches_models: bool, tick: u64) {
        let state = &self.invariants;
        match branch {
            None => {
                state.baseline.fetch_max(tick, Ordering::AcqRel);
            }
            Some(branch) => {
                let mut branches = state
                    .branches
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let clock = branches.entry(branch.to_owned()).or_default();
                *clock = (*clock).max(tick);
            }
        }
        if touches_models {
            state.carriers.fetch_max(tick, Ordering::AcqRel);
        }
    }

    /// The clocks a verdict for `scope` pins. Read them before any state the
    /// write's admission reads, so a write landing in between makes the
    /// verdict stale rather than wrong.
    pub(super) fn invariant_pins(&self, scope: InvariantScope<'_>) -> Generations {
        let state = &self.invariants;
        Generations {
            carriers: state.carriers.load(Ordering::Acquire),
            baseline: state.baseline.load(Ordering::Acquire),
            branch: scope.branch().map(|b| state.branch_clock(b)),
        }
    }

    async fn registry(&self, branch: Option<&str>) -> Result<Registry, Status> {
        let state = &self.invariants;
        let epoch = state.carriers.load(Ordering::Acquire);
        let key = branch.map(str::to_owned);
        if let Some((at, registry)) = state.registry.lock().await.get(&key) {
            if *at == epoch {
                return Ok(registry.clone());
            }
        }
        let models = self
            .store
            .list(
                ListFilter {
                    kinds: &[pb::EntityKind::EventModel],
                    namespaces: &[],
                    latest_versions_only: false,
                },
                Some(MAX_REGISTRY_MODELS + 1),
                branch,
            )
            .await
            .map_err(store_err)?;
        let registry = if models.len() > MAX_REGISTRY_MODELS {
            Registry::Unbounded
        } else {
            Registry::Carriers(Arc::new(
                models
                    .into_iter()
                    .filter(|m| invariants::declares_invariants(&m.entity))
                    .collect(),
            ))
        };
        state
            .registry
            .lock()
            .await
            .insert(key, (epoch, registry.clone()));
        Ok(registry)
    }

    /// Judges `changes` against a snapshot of `scope`, outside the mutation
    /// lock. `Ok(None)` when no invariant is declared in scope or by the
    /// change; a typed refusal when one blocks it.
    pub(super) async fn check_invariants(
        &self,
        scope: InvariantScope<'_>,
        changes: &[Change],
        vis: &Visibility,
    ) -> Result<Option<pb::InvariantCheck>, Status> {
        let registry = self.registry(scope.branch()).await?;
        let declared_by_change = changes
            .iter()
            .any(|(_, _, e)| e.as_ref().is_some_and(invariants::declares_invariants));
        if !declared_by_change && !registry.declares() {
            return Ok(None);
        }
        let unrestricted = Visibility::unrestricted(vis.principal().into());
        let (before, truncated) = self
            .snapshot_with_truncation(scope.world(), &unrestricted)
            .await?;
        let change_refs: Vec<(pb::EntityKind, &pb::Id, Option<&pb::Entity>)> = changes
            .iter()
            .map(|(kind, id, entity)| (*kind, id, entity.as_ref()))
            .collect();
        let after = world_after(&before, &change_refs);
        let mut outcomes = invariants::evaluate(
            &Worlds {
                before: &before,
                after: &after,
                truncated,
            },
            self.invariants.budget,
        );
        if truncated {
            outcomes.extend(unseen_carriers(&registry, &after, changes));
        }

        let lens = self.lens(vis).await?;
        let admitted = |r: &pb::EntityRef| {
            r.id.as_ref()
                .is_some_and(|id| lens.admits(id.namespace.as_str()))
        };
        for outcome in &mut outcomes {
            if !outcome.carrier.as_ref().is_some_and(admitted) {
                invariants::redact(outcome);
            }
        }
        let inputs = inputs(&before, &outcomes, changes)
            .into_iter()
            .filter(|input| input.r#ref.as_ref().is_some_and(admitted))
            .collect();
        let snapshot_id = snapshot_digest(&before, &BTreeSet::new(), false)
            .map(|digest| digest.snapshot_id)
            .unwrap_or_default();
        if let Some(refusal) = invariants::refusal(&outcomes) {
            return Err(conv::invariant_refused(refusal, &snapshot_id));
        }
        Ok(Some(pb::InvariantCheck {
            complete: invariants::complete(&outcomes) && !snapshot_id.is_empty(),
            outcomes,
            snapshot_id,
            inputs,
        }))
    }

    /// The ticket for a write no invariant can see, so a route can skip
    /// assembling its change set when nothing in scope declares one.
    pub(super) async fn unguarded(
        &self,
        scope: InvariantScope<'_>,
        generations: Generations,
        declared_by_change: bool,
    ) -> Result<Option<InvariantTicket>, Status> {
        if declared_by_change || self.registry(scope.branch()).await?.declares() {
            return Ok(None);
        }
        Ok(Some(InvariantTicket::Unguarded {
            carriers: generations.carriers,
        }))
    }

    /// Judges a write whose change set the request already names.
    pub(super) async fn ticket_for_changes(
        &self,
        scope: InvariantScope<'_>,
        generations: Generations,
        changes: &[Change],
        vis: &Visibility,
    ) -> Result<InvariantTicket, Status> {
        let declared_by_change = changes
            .iter()
            .any(|(_, _, e)| e.as_ref().is_some_and(invariants::declares_invariants));
        if let Some(ticket) = self
            .unguarded(scope, generations, declared_by_change)
            .await?
        {
            return Ok(ticket);
        }
        let verdict = match self.check_invariants(scope, changes, vis).await {
            Err(status) if !conv::is_invariant_refusal(&status) => return Err(status),
            verdict => verdict,
        };
        Ok(InvariantTicket::Judged {
            generations,
            verdict,
        })
    }

    /// Judges a write whose change set only the route itself computes, by
    /// running the route's own dry run as `preview` first.
    pub(super) async fn ticket_by_preview<T, R, Fut>(
        &self,
        scope: InvariantScope<'_>,
        preview: Request<T>,
        run: impl FnOnce(Request<T>) -> Fut,
        check_of: impl FnOnce(&R) -> Option<pb::InvariantCheck>,
    ) -> Result<InvariantTicket, Status>
    where
        Fut: Future<Output = Result<Response<R>, Status>>,
    {
        let generations = self.invariant_pins(scope);
        if let Some(ticket) = self.unguarded(scope, generations, false).await? {
            return Ok(ticket);
        }
        Ok(match run(preview).await {
            Ok(response) => InvariantTicket::Judged {
                generations,
                verdict: Ok(check_of(response.get_ref())),
            },
            Err(status) if conv::is_invariant_refusal(&status) => InvariantTicket::Judged {
                generations,
                verdict: Err(status),
            },
            Err(_) => InvariantTicket::Unjudged,
        })
    }

    /// The cheap half of the gate, run under the mutation lock: the verdict
    /// stands only if no clock it pinned has moved.
    pub(super) fn invariant_gate(
        &self,
        scope: InvariantScope<'_>,
        ticket: &InvariantTicket,
    ) -> Result<Option<pb::InvariantCheck>, Status> {
        match ticket {
            InvariantTicket::Unguarded { carriers }
                if *carriers == self.invariants.carriers.load(Ordering::Acquire) =>
            {
                Ok(None)
            }
            InvariantTicket::Judged {
                generations,
                verdict,
            } if *generations == self.invariant_pins(scope) => verdict.clone(),
            InvariantTicket::Unguarded { .. }
            | InvariantTicket::Judged { .. }
            | InvariantTicket::Unjudged => Err(conv::invariant_stale()),
        }
    }
}

/// Outcomes for carriers a truncated snapshot did not load. When the
/// carriers themselves could not be enumerated, one unattributed outcome
/// stands for all of them.
fn unseen_carriers(
    registry: &Registry,
    after: &[StoredEntity],
    changes: &[Change],
) -> Vec<pb::InvariantOutcome> {
    use pb::invariant_outcome::{Reason, Status as Verdict};
    let truncated = |carrier: Option<pb::EntityRef>, name: String, advisory: bool| {
        let mut outcome = pb::InvariantOutcome {
            carrier,
            name,
            advisory,
            ..Default::default()
        };
        outcome.set_status(Verdict::Unevaluable);
        outcome.set_reason(Reason::SnapshotTruncated);
        outcome
    };
    let carriers = match registry {
        Registry::Unbounded => return vec![truncated(None, String::new(), false)],
        Registry::Carriers(carriers) => carriers,
    };
    let seen: HashSet<pb::EntityKey> = after
        .iter()
        .filter_map(|se| {
            Some(pb::EntityKey::new(
                entity_kind(&se.entity).ok()?,
                entity_id(&se.entity).ok()?,
            ))
        })
        .chain(
            changes
                .iter()
                .map(|(kind, id, _)| pb::EntityKey::new(*kind, id)),
        )
        .collect();
    let mut outcomes = Vec::new();
    for carrier in carriers.iter() {
        let (Ok(kind), Ok(id)) = (entity_kind(&carrier.entity), entity_id(&carrier.entity)) else {
            continue;
        };
        if seen.contains(&pb::EntityKey::new(kind, id)) {
            continue;
        }
        let entity_ref = pb::EntityRef {
            kind: kind as i32,
            id: Some(id.clone()),
        };
        for (name, advisory) in invariants::declared_names(&carrier.entity) {
            outcomes.push(truncated(Some(entity_ref.clone()), name, advisory));
        }
    }
    outcomes
}

/// The carriers and touched entities a verdict read, with their etags in
/// the snapshot it read them from.
fn inputs(
    before: &[StoredEntity],
    outcomes: &[pb::InvariantOutcome],
    changes: &[Change],
) -> Vec<pb::InvariantInput> {
    let etags: HashMap<pb::EntityKey, &str> = before
        .iter()
        .filter_map(|se| {
            let key =
                pb::EntityKey::new(entity_kind(&se.entity).ok()?, entity_id(&se.entity).ok()?);
            Some((key, se.etag.as_str()))
        })
        .collect();
    let mut seen = HashSet::new();
    let refs = outcomes
        .iter()
        .filter_map(|o| o.carrier.clone())
        .chain(changes.iter().map(|(kind, id, _)| pb::EntityRef {
            kind: *kind as i32,
            id: Some(id.clone()),
        }));
    refs.filter_map(|r| {
        let kind = pb::EntityKind::try_from(r.kind).ok()?;
        let key = pb::EntityKey::new(kind, r.id.as_ref()?);
        let etag = etags.get(&key).copied().unwrap_or_default().to_owned();
        seen.insert(key).then_some(pb::InvariantInput {
            r#ref: Some(r),
            etag,
        })
    })
    .collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::sync::Arc;

    use tonic::Code;
    use tonic_types::StatusExt as _;

    use super::{InvariantScope, InvariantTicket};
    use crate::{metrics::ServerMetrics, service::EventModelServiceImpl};

    async fn service() -> EventModelServiceImpl {
        let nats = trogon_atlas_testsupport::shared().await;
        EventModelServiceImpl::try_new(Arc::new(nats.store().await), ServerMetrics::default())
            .unwrap()
    }

    fn judged(service: &EventModelServiceImpl, scope: InvariantScope<'_>) -> InvariantTicket {
        InvariantTicket::Judged {
            generations: service.invariant_pins(scope),
            verdict: Ok(None),
        }
    }

    fn assert_stale(result: Result<Option<super::pb::InvariantCheck>, tonic::Status>) {
        let status = result.expect_err("the verdict was taken against a moved world");
        assert_eq!(status.code(), Code::Aborted);
        assert_eq!(
            status.get_details_error_info().unwrap().reason,
            "INVARIANT_STALE"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_verdict_stands_while_nothing_it_pinned_moved() {
        let service = service().await;
        let ticket = judged(&service, InvariantScope::Branch("ia-quiet"));
        service.note_write(Some("ia-elsewhere"), false);
        assert!(service
            .invariant_gate(InvariantScope::Branch("ia-quiet"), &ticket)
            .is_ok());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_write_in_the_judged_scope_makes_the_verdict_stale() {
        let service = service().await;
        let baseline = judged(&service, InvariantScope::Baseline);
        let branch = judged(&service, InvariantScope::Branch("ia-moved"));
        service.note_write(Some("ia-moved"), false);
        assert!(service
            .invariant_gate(InvariantScope::Baseline, &baseline)
            .is_ok());
        assert_stale(service.invariant_gate(InvariantScope::Branch("ia-moved"), &branch));

        let on_branch = judged(&service, InvariantScope::Branch("ia-under"));
        service.note_write(None, false);
        assert_stale(service.invariant_gate(InvariantScope::Branch("ia-under"), &on_branch));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_new_carrier_makes_an_unguarded_write_stale() {
        let service = service().await;
        let generations = service.invariant_pins(InvariantScope::Baseline);
        let ticket = service
            .unguarded(InvariantScope::Baseline, generations, false)
            .await
            .unwrap()
            .expect("an empty scope declares nothing");
        service.note_write(Some("ia-other"), true);
        assert_stale(service.invariant_gate(InvariantScope::Baseline, &ticket));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_recreated_branch_never_matches_old_pins() {
        let service = service().await;
        let ticket = judged(&service, InvariantScope::Landing("ia-recreated"));
        service.note_write(Some("ia-recreated"), true);
        assert_stale(service.invariant_gate(InvariantScope::Landing("ia-recreated"), &ticket));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_late_tick_never_moves_a_clock_backwards() {
        let service = service().await;
        for branch in [None, Some("ia-late")] {
            let scope = InvariantScope::of(branch);
            let early = service.invariants.next_tick();
            service.note_write(branch, true);
            let ticket = judged(&service, scope);
            service.publish_tick(branch, true, early);
            assert!(service.invariant_gate(scope, &ticket).is_ok());
            service.note_write(branch, true);
            assert_stale(service.invariant_gate(scope, &ticket));
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_unjudged_write_is_always_stale() {
        let service = service().await;
        assert_stale(service.invariant_gate(InvariantScope::Baseline, &InvariantTicket::Unjudged));
    }
}
