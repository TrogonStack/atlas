use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::{atomic::Ordering, Arc},
};

use chrono::Utc;
use prost::Message;
use sha2::{Digest, Sha256};
use tonic::{Request, Response};
use trogon_atlas_core::{authored_date::AuthoredDate, glossary as core, transcode::TranscodePool};
use trogon_atlas_store::store::BranchLandOp;

use super::{
    pb, store_err, EventModelServiceImpl, ListFilter, OpMetaLike, Status, StoredEntity, Visibility,
};

const MAX_ROWS: usize = 5_000;
const MAX_CATALOG_BYTES: usize = 32 * 1024 * 1024;
const MAX_EVIDENCE_WORK: usize = 10_000;
const MAX_EVIDENCE_BYTES: usize = 32 * 1024 * 1024;

tokio::task_local! {
    static REVIEW_PERMIT: pb::Entity;
}

#[derive(Default)]
struct Budget {
    work: usize,
    bytes: usize,
    charged_documents: HashSet<pb::EntityKey>,
    documents: HashMap<pb::IdKey, Result<Arc<ResolvedDocument>, Status>>,
    pools: HashMap<String, TranscodePool>,
}

struct ResolvedDocument {
    source: core::EvidenceDocument,
    etag: String,
    provenance: [u8; 32],
}

impl ResolvedDocument {
    fn new(stored: StoredEntity) -> Self {
        let mut hash = Sha256::new();
        hash.update(b"glossary-document-v1");
        hash.update(Sha256::digest(stored.entity.encode_to_vec()));
        hash.update(stored.etag.as_bytes());
        Self {
            source: core::EvidenceDocument::new(stored.entity),
            etag: stored.etag,
            provenance: hash.finalize().into(),
        }
    }
}

impl Budget {
    fn attempt(&mut self) -> Result<(), Status> {
        self.work = self.work.saturating_add(1);
        if self.work > MAX_EVIDENCE_WORK {
            return Err(Status::resource_exhausted(
                "glossary evidence budget exceeded",
            ));
        }
        Ok(())
    }

    fn source(&mut self, entity: &pb::Entity) -> Result<(), Status> {
        if matches!(entity.kind, Some(pb::entity::Kind::Document(_))) {
            let key = pb::EntityKey::new(pb::EntityKind::Document, super::entity_id(entity)?);
            if !self.charged_documents.insert(key) {
                return Ok(());
            }
        }
        self.bytes = self.bytes.saturating_add(entity.encoded_len());
        if self.bytes > MAX_EVIDENCE_BYTES {
            return Err(Status::resource_exhausted(
                "glossary evidence budget exceeded",
            ));
        }
        Ok(())
    }
}

pub(super) struct GlossaryReadContext {
    pub(super) visibility: Visibility,
    pub(super) branch: Option<String>,
    incarnation: Option<String>,
    pins: super::invariant_admission::Generations,
    snapshot_generation: u64,
    namespace_generation: u64,
    store_marker: trogon_atlas_store::StoreReadMarker,
}

struct Entry {
    stored: StoredEntity,
    index: pb::GlossaryIndexEntry,
    evidence: Vec<pb::GlossaryEvidenceResolution>,
    incomplete: bool,
    provenance: Vec<u8>,
}

struct Page {
    entries: Vec<Entry>,
    next_page_token: String,
    revision: String,
    incomplete: bool,
}

fn pointer(index: pb::GlossaryIndexEntry) -> pb::GlossaryPointer {
    pb::GlossaryPointer {
        term: index.term,
        title: index.title,
        audience: index.audience,
        state: index.state,
    }
}

struct Catalog {
    entries: Vec<Entry>,
    revision: String,
    incomplete: bool,
}

fn invalid(error: impl std::fmt::Display) -> Status {
    Status::invalid_argument(error.to_string())
}

fn term(entity: &pb::Entity) -> Option<&pb::Term> {
    match entity.kind.as_ref() {
        Some(pb::entity::Kind::Term(value)) => Some(value),
        _ => None,
    }
}

fn uid(entity: &pb::Entity) -> &str {
    entity
        .system
        .as_ref()
        .map_or("", |system| system.uid.as_str())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut output, byte| {
        use std::fmt::Write as _;
        let _ = write!(output, "{byte:02x}");
        output
    })
}

fn today() -> String {
    Utc::now().format("%Y-%m-%d").to_string()
}

fn bounded_output<M: Message>(name: &str, message: M) -> Result<M, Status> {
    let pool = trogon_atlas_core::transcode::TranscodePool::builtin()
        .map_err(|_| Status::internal("native glossary descriptors are unavailable"))?;
    bounded_output_with_pool(name, message, &pool)
}

fn bounded_output_with_pool<M: Message>(
    name: &str,
    message: M,
    pool: &trogon_atlas_core::transcode::TranscodePool,
) -> Result<M, Status> {
    if message.encoded_len() > 3 * 1024 * 1024 {
        return Err(Status::resource_exhausted(
            "glossary response exceeds 3 MiB",
        ));
    }
    let value = pool
        .message_json_value(
            &format!("trogonatlas.api.eventmodel.v1alpha1.{name}"),
            &message,
        )
        .map_err(|_| Status::internal("glossary response cannot be represented losslessly"))?;
    let json = serde_json::to_vec(&value)
        .map_err(|_| Status::internal("glossary response cannot be encoded"))?;
    if json.len() > 3 * 1024 * 1024 {
        return Err(Status::resource_exhausted(
            "glossary JSON response exceeds 3 MiB",
        ));
    }
    Ok(message)
}

fn exact(id: Option<&pb::Id>) -> Result<&pb::Id, Status> {
    let id = super::require_id(id, "exact glossary reference")?;
    if id.version == 0 {
        return Err(Status::invalid_argument(
            "glossary references require a positive version",
        ));
    }
    Ok(id)
}

impl EventModelServiceImpl {
    async fn glossary_pool(
        &self,
        namespace: &str,
        changes: &crate::type_libraries::TypeLibraryChanges,
        branch: Option<&str>,
        budget: &mut Budget,
    ) -> Result<TranscodePool, Status> {
        if let Some(pool) = budget.pools.get(namespace) {
            return Ok(pool.clone());
        }
        let pool = self
            .transcode_pool_after_changes(namespace, changes, branch)
            .await?;
        budget.pools.insert(namespace.into(), pool.clone());
        Ok(pool)
    }

    pub(super) async fn glossary_context<T>(
        &self,
        request: &Request<T>,
    ) -> Result<(Visibility, Option<String>), Status> {
        let visibility = Self::visibility_of(request);
        let branch = Self::branch_from_metadata(request)?;
        let principal = Self::principal_from_metadata(request);
        if let Some(branch) = branch.as_deref() {
            if !Self::branch_visible(principal.as_ref(), branch) {
                return Err(Status::not_found("branch not found"));
            }
            self.require_branch(branch).await?;
        }
        Ok((visibility, branch))
    }

    pub(super) async fn begin_glossary_read<T>(
        &self,
        request: &Request<T>,
    ) -> Result<GlossaryReadContext, Status> {
        let store_marker = self.store.read_marker().await.map_err(store_err)?;
        if !Self::visibility_of(request).is_unrestricted() {
            let mut namespace_marker = self.glossary_namespace_marker.lock().await;
            if namespace_marker.as_ref() != Some(store_marker.namespace_marker()) {
                self.directory.refresh(&*self.store).await?;
                self.owner_snapshot_cache.write().await.clear();
                self.owner_reverse_index_cache.write().await.clear();
                *namespace_marker = Some(store_marker.namespace_marker().clone());
            }
        }
        let (visibility, branch) = self.glossary_context(request).await?;
        let incarnation = if let Some(branch) = branch.as_deref() {
            Some(self.require_branch(branch).await?.created_at)
        } else {
            None
        };
        Ok(GlossaryReadContext {
            pins: self.invariant_pins(super::InvariantScope::of(branch.as_deref())),
            snapshot_generation: self.snapshot_generation.load(Ordering::Acquire),
            namespace_generation: self.namespace_generation.load(Ordering::Acquire),
            store_marker,
            visibility,
            branch,
            incarnation,
        })
    }

    pub(super) async fn finish_glossary_read<T>(
        &self,
        context: &GlossaryReadContext,
        result: Result<T, Status>,
    ) -> Result<T, Status> {
        let incarnation = if let Some(branch) = context.branch.as_deref() {
            Some(self.require_branch(branch).await?.created_at)
        } else {
            None
        };
        let store_marker = self.store.read_marker().await.map_err(store_err)?;
        if store_marker != context.store_marker
            || incarnation != context.incarnation
            || self.invariant_pins(super::InvariantScope::of(context.branch.as_deref()))
                != context.pins
            || self.snapshot_generation.load(Ordering::Acquire) != context.snapshot_generation
            || self.namespace_generation.load(Ordering::Acquire) != context.namespace_generation
        {
            return Err(Status::aborted("glossary changed while reading; retry"));
        }
        result
    }

    async fn glossary_row(
        &self,
        kind: pb::EntityKind,
        id: &pb::Id,
        branch: Option<&str>,
        visibility: &Visibility,
    ) -> Result<StoredEntity, Status> {
        self.require_visible(visibility, &id.namespace).await?;
        self.store.get(kind, id, branch).await.map_err(store_err)
    }

    async fn glossary_source(
        &self,
        id: &pb::Id,
        branch: Option<&str>,
        visibility: &Visibility,
        budget: &mut Budget,
    ) -> Result<Arc<ResolvedDocument>, Status> {
        budget.attempt()?;
        let key = pb::IdKey::new(id);
        if let Some(cached) = budget.documents.get(&key) {
            return cached.clone();
        }
        let resolved = match self
            .glossary_row(pb::EntityKind::Document, id, branch, visibility)
            .await
        {
            Ok(row) => {
                budget.source(&row.entity)?;
                Ok(Arc::new(ResolvedDocument::new(row)))
            }
            Err(error) if error.code() == tonic::Code::NotFound => Err(error),
            Err(error) => return Err(error),
        };
        budget.documents.insert(key, resolved.clone());
        resolved
    }

    async fn glossary_entry(
        &self,
        stored: StoredEntity,
        branch: Option<&str>,
        visibility: &Visibility,
        budget: &mut Budget,
    ) -> Result<Entry, Status> {
        let value = term(&stored.entity)
            .ok_or_else(|| Status::internal("stored glossary kind is invalid"))?;
        core::validate_term(value)
            .map_err(|_| Status::internal("stored glossary values are invalid"))?;
        let mut state = pb::GlossaryEntryState::Draft;
        let mut reason = "No current expert review".to_owned();
        let review_current = if value.metadata.is_empty() {
            core::review_is_current(value, uid(&stored.entity))
        } else if value
            .glossary
            .as_ref()
            .is_some_and(|entry| entry.review.is_some())
        {
            let namespace = &exact(value.id.as_ref())?.namespace;
            let pool = self
                .glossary_pool(namespace, &Default::default(), branch, budget)
                .await?;
            core::review_is_current_with_pool(value, uid(&stored.entity), &pool)
        } else {
            false
        };
        let published = review_current
            && value
                .glossary
                .as_ref()
                .and_then(|entry| entry.review.as_ref())
                .is_some_and(|review| {
                    review.disposition == pb::GlossaryReviewDisposition::Accepted as i32
                });
        let mut incomplete = false;
        let mut evidence = Vec::new();
        let mut hash = Sha256::new();
        hash.update(stored.entity.encode_to_vec());
        hash.update(stored.etag.as_bytes());
        if let Some(glossary) = &value.glossary {
            if let Some(review) = &glossary.review {
                if review_current {
                    state = match pb::GlossaryReviewDisposition::try_from(review.disposition)
                        .map_err(invalid)?
                    {
                        pb::GlossaryReviewDisposition::Accepted => pb::GlossaryEntryState::Reviewed,
                        pb::GlossaryReviewDisposition::Rejected => pb::GlossaryEntryState::Rejected,
                        pb::GlossaryReviewDisposition::NeedsChanges => {
                            pb::GlossaryEntryState::NeedsChanges
                        }
                        pb::GlossaryReviewDisposition::Unspecified => {
                            pb::GlossaryEntryState::StaleReview
                        }
                    };
                    reason = review.reason.clone();
                } else {
                    state = pb::GlossaryEntryState::StaleReview;
                    reason = "Review does not bind this content and incarnation".into();
                }
            }
            for (index, selection) in glossary.evidence.iter().enumerate() {
                let id = exact(
                    selection
                        .document
                        .as_ref()
                        .and_then(|reference| reference.id.as_ref()),
                )?;
                let (source_state, source_reason) =
                    match self.glossary_source(id, branch, visibility, budget).await {
                        Ok(row) => {
                            hash.update(row.provenance);
                            if row.source.verify(selection).is_ok() {
                                (
                                    pb::GlossaryEvidenceState::Current,
                                    "Exact source evidence resolves",
                                )
                            } else {
                                (
                                    pb::GlossaryEvidenceState::Changed,
                                    "Source incarnation, content or quoted passage changed",
                                )
                            }
                        }
                        Err(error) if error.code() == tonic::Code::NotFound => {
                            incomplete = true;
                            hash.update(b"unavailable");
                            (
                                pb::GlossaryEvidenceState::Unavailable,
                                "Source is unavailable",
                            )
                        }
                        Err(error) => return Err(error),
                    };
                evidence.push(pb::GlossaryEvidenceResolution {
                    evidence_index: u32::try_from(index).map_err(invalid)?,
                    state: source_state as i32,
                    reason: source_reason.into(),
                });
            }
            if published {
                if evidence
                    .iter()
                    .any(|item| item.state == pb::GlossaryEvidenceState::Unavailable as i32)
                {
                    state = pb::GlossaryEntryState::SourceUnavailable;
                    reason = "Published definition has unavailable evidence".into();
                } else if evidence
                    .iter()
                    .any(|item| item.state == pb::GlossaryEvidenceState::Changed as i32)
                {
                    state = pb::GlossaryEntryState::SourceChanged;
                    reason = "Published definition requires source reconciliation".into();
                } else if glossary
                    .next_review_on
                    .as_ref()
                    .is_some_and(|date| date.value <= today())
                {
                    state = pb::GlossaryEntryState::ReviewDue;
                    reason = "The declared review date is due".into();
                }
            }
        }
        let index = pb::GlossaryIndexEntry {
            term: Some(pb::TermRef {
                id: value.id.clone(),
            }),
            owner_uid: uid(&stored.entity).into(),
            etag: stored.etag.clone(),
            title: value.title.clone(),
            audience: value
                .glossary
                .as_ref()
                .map_or_else(String::new, |value| value.audience.clone()),
            state: state as i32,
            published,
            reason,
            next_review_on: value
                .glossary
                .as_ref()
                .and_then(|value| value.next_review_on.clone()),
        };
        hash.update(index.encode_to_vec());
        Ok(Entry {
            stored,
            index,
            evidence,
            incomplete,
            provenance: hash.finalize().to_vec(),
        })
    }

    async fn glossary_catalog(
        &self,
        selection: pb::GlossarySelection,
        branch: Option<&str>,
        visibility: &Visibility,
        budget: &mut Budget,
    ) -> Result<Catalog, Status> {
        if selection.namespaces.is_empty()
            || selection.namespaces.len() > 32
            || selection.audience.len() > 256
        {
            return Err(Status::invalid_argument(
                "select 1 to 32 glossary namespaces and a bounded audience",
            ));
        }
        let mut namespaces = selection.namespaces.clone();
        namespaces.sort();
        namespaces.dedup();
        for namespace in &namespaces {
            trogon_atlas_core::validate_id_component(namespace).map_err(invalid)?;
            self.require_visible(visibility, namespace).await?;
        }
        let mut rows = self
            .store
            .list_bounded(
                ListFilter {
                    kinds: &[pb::EntityKind::Term],
                    namespaces: &namespaces,
                    ..Default::default()
                },
                Some(MAX_ROWS + 1),
                branch,
                trogon_atlas_store::EntityByteBudget::new(MAX_CATALOG_BYTES),
            )
            .await
            .map_err(store_err)?;
        if rows.len() > MAX_ROWS {
            return Err(Status::resource_exhausted(
                "glossary catalog exceeds its row budget",
            ));
        }
        rows.sort_by_key(|row| {
            term(&row.entity)
                .and_then(|value| value.id.clone())
                .map(|id| (id.namespace, id.slug, id.version))
        });
        let mut hash = Sha256::new();
        hash.update(format!("{visibility:?}:{branch:?}"));
        hash.update(selection.encode_to_vec());
        hash.update(today());
        let mut entries = Vec::new();
        for row in rows {
            let value = term(&row.entity)
                .ok_or_else(|| Status::internal("glossary row kind is invalid"))?;
            let audience = value
                .glossary
                .as_ref()
                .map_or("", |value| value.audience.as_str());
            if !selection.audience.is_empty() && audience != selection.audience {
                continue;
            }
            let entry = self.glossary_entry(row, branch, visibility, budget).await?;
            hash.update(&entry.provenance);
            if selection.include_unreviewed || entry.index.published {
                entries.push(entry);
            }
        }
        if !selection.include_unreviewed {
            let mut latest = BTreeMap::new();
            for entry in entries {
                let id = exact(
                    entry
                        .index
                        .term
                        .as_ref()
                        .and_then(|value| value.id.as_ref()),
                )?;
                latest.insert(
                    (
                        id.namespace.clone(),
                        id.slug.clone(),
                        entry.index.audience.clone(),
                    ),
                    entry,
                );
            }
            entries = latest.into_values().collect();
        }
        let incomplete = entries.iter().any(|entry| entry.incomplete);
        Ok(Catalog {
            entries,
            incomplete,
            revision: hex(&hash.finalize()),
        })
    }

    async fn glossary_page(
        &self,
        request: pb::ListGlossaryEntriesRequest,
        branch: Option<&str>,
        visibility: &Visibility,
    ) -> Result<Page, Status> {
        if request.page_size > 500 || request.page_token.len() > 128 {
            return Err(Status::invalid_argument("invalid glossary page"));
        }
        let mut budget = Budget::default();
        let catalog = self
            .glossary_catalog(
                request
                    .selection
                    .ok_or_else(|| Status::invalid_argument("selection is required"))?,
                branch,
                visibility,
                &mut budget,
            )
            .await?;
        let offset = if request.page_token.is_empty() {
            0
        } else {
            let (revision, offset) = request
                .page_token
                .split_once(':')
                .ok_or_else(|| Status::invalid_argument("invalid glossary page token"))?;
            if revision != catalog.revision {
                return Err(Status::failed_precondition(
                    "glossary page token is stale or has a different scope",
                ));
            }
            offset.parse::<usize>().map_err(invalid)?
        };
        if offset > catalog.entries.len() {
            return Err(Status::invalid_argument("invalid glossary page offset"));
        }
        let size = if request.page_size == 0 {
            100
        } else {
            request.page_size as usize
        };
        let end = offset.saturating_add(size).min(catalog.entries.len());
        let next_page_token = if end < catalog.entries.len() {
            format!("{}:{end}", catalog.revision)
        } else {
            String::new()
        };
        Ok(Page {
            entries: catalog
                .entries
                .into_iter()
                .skip(offset)
                .take(size)
                .collect(),
            next_page_token,
            incomplete: catalog.incomplete,
            revision: catalog.revision,
        })
    }

    pub(super) async fn glossary_list(
        &self,
        request: pb::ListGlossaryEntriesRequest,
        branch: Option<&str>,
        visibility: &Visibility,
    ) -> Result<pb::ListGlossaryEntriesResponse, Status> {
        let page = self.glossary_page(request, branch, visibility).await?;
        bounded_output(
            "ListGlossaryEntriesResponse",
            pb::ListGlossaryEntriesResponse {
                entries: page.entries.into_iter().map(|entry| entry.index).collect(),
                next_page_token: page.next_page_token,
                incomplete: page.incomplete,
                snapshot_revision: page.revision,
            },
        )
    }

    pub(super) async fn glossary_pointers(
        &self,
        request: pb::ListGlossaryEntriesRequest,
        branch: Option<&str>,
        visibility: &Visibility,
    ) -> Result<pb::ListGlossaryPointersResponse, Status> {
        if request
            .selection
            .as_ref()
            .is_some_and(|selection| selection.include_unreviewed)
        {
            return Err(Status::invalid_argument(
                "glossary pointers require published selection",
            ));
        }
        let page = self.glossary_page(request, branch, visibility).await?;
        bounded_output(
            "ListGlossaryPointersResponse",
            pb::ListGlossaryPointersResponse {
                entries: page
                    .entries
                    .into_iter()
                    .map(|entry| pointer(entry.index))
                    .collect(),
                next_page_token: page.next_page_token,
                incomplete: page.incomplete,
                snapshot_revision: page.revision,
            },
        )
    }

    async fn glossary_read(
        &self,
        request: pb::GetGlossaryTermRequest,
        branch: Option<&str>,
        visibility: &Visibility,
    ) -> Result<Entry, Status> {
        let id = exact(
            request
                .term
                .as_ref()
                .and_then(|reference| reference.id.as_ref()),
        )?;
        let row = self
            .glossary_row(pb::EntityKind::Term, id, branch, visibility)
            .await?;
        self.glossary_entry(row, branch, visibility, &mut Budget::default())
            .await
    }

    pub(super) async fn glossary_detail(
        &self,
        request: pb::GetGlossaryTermRequest,
        branch: Option<&str>,
        visibility: &Visibility,
    ) -> Result<pb::GetGlossaryTermResponse, Status> {
        let entry = self.glossary_read(request, branch, visibility).await?;
        let namespace = &super::entity_id(&entry.stored.entity)?.namespace;
        let pool = self.transcode_pool(namespace, branch).await?;
        bounded_output_with_pool(
            "GetGlossaryTermResponse",
            pb::GetGlossaryTermResponse {
                entity: Some(entry.stored.entity),
                etag: entry.stored.etag,
                entry: Some(entry.index),
                evidence: entry.evidence,
                incomplete: entry.incomplete,
            },
            &pool,
        )
    }

    pub(super) async fn glossary_definition(
        &self,
        request: pb::GetGlossaryTermRequest,
        branch: Option<&str>,
        visibility: &Visibility,
    ) -> Result<pb::GetGlossaryDefinitionResponse, Status> {
        let entry = self.glossary_read(request, branch, visibility).await?;
        let term = term(&entry.stored.entity)
            .ok_or_else(|| Status::internal("stored glossary kind is invalid"))?;
        bounded_output(
            "GetGlossaryDefinitionResponse",
            pb::GetGlossaryDefinitionResponse {
                owner_uid: uid(&entry.stored.entity).into(),
                etag: entry.stored.etag.clone(),
                snapshot_revision: hex(&entry.provenance),
                definition: term.doc.clone(),
                policy: term
                    .glossary
                    .as_ref()
                    .and_then(|glossary| glossary.policy.clone()),
                usage_period: term
                    .glossary
                    .as_ref()
                    .and_then(|glossary| glossary.usage_period.clone()),
                entry: Some(pointer(entry.index)),
                evidence: entry.evidence,
                incomplete: entry.incomplete,
            },
        )
    }

    pub(super) async fn glossary_sources(
        &self,
        request: pb::InspectGlossarySourcesRequest,
        branch: Option<&str>,
        visibility: &Visibility,
    ) -> Result<pb::InspectGlossarySourcesResponse, Status> {
        let mut budget = Budget::default();
        self.glossary_sources_with_budget(request, branch, visibility, &mut budget)
            .await
    }

    async fn glossary_sources_with_budget(
        &self,
        request: pb::InspectGlossarySourcesRequest,
        branch: Option<&str>,
        visibility: &Visibility,
        budget: &mut Budget,
    ) -> Result<pb::InspectGlossarySourcesResponse, Status> {
        if request.documents.len() > 32 {
            return Err(Status::invalid_argument(
                "at most 32 exact source Documents are supported",
            ));
        }
        let mut sources = Vec::new();
        let mut bytes = 0usize;
        let mut incomplete = false;
        let mut hash = Sha256::new();
        hash.update(format!("{visibility:?}:{branch:?}"));
        for reference in request.documents {
            let id = exact(reference.id.as_ref())?;
            hash.update(reference.encode_to_vec());
            match self.glossary_source(id, branch, visibility, budget).await {
                Ok(row) => {
                    let content =
                        core::retained_document_bytes(row.source.entity()).map_err(invalid)?;
                    bytes = bytes.saturating_add(content.len());
                    if bytes > 1024 * 1024 {
                        return Err(Status::resource_exhausted(
                            "glossary source text exceeds 1 MiB",
                        ));
                    }
                    let text = std::str::from_utf8(content)
                        .map_err(|_| {
                            Status::invalid_argument("glossary sources require retained UTF-8 text")
                        })?
                        .to_owned();
                    hash.update(row.provenance);
                    sources.push(pb::GlossarySourceMaterial {
                        document: Some(reference),
                        owner_uid: uid(row.source.entity()).into(),
                        etag: row.etag.clone(),
                        content_digest: Some(row.source.content_digest().map_err(invalid)?.clone()),
                        text,
                        state: pb::GlossaryEvidenceState::Current as i32,
                    });
                }
                Err(error) if error.code() == tonic::Code::NotFound => {
                    incomplete = true;
                    hash.update(b"unavailable");
                    sources.push(pb::GlossarySourceMaterial {
                        document: Some(reference),
                        state: pb::GlossaryEvidenceState::Unavailable as i32,
                        ..Default::default()
                    });
                }
                Err(error) => return Err(error),
            }
        }
        bounded_output(
            "InspectGlossarySourcesResponse",
            pb::InspectGlossarySourcesResponse {
                sources,
                incomplete,
                snapshot_revision: hex(&hash.finalize()),
            },
        )
    }

    pub(super) async fn glossary_review(
        &self,
        request: Request<pb::ReviewGlossaryTermRequest>,
    ) -> Result<Response<pb::GetGlossaryTermResponse>, Status> {
        use pb::event_model_service_server::EventModelService;
        let principal = Self::principal_from_metadata(&request)
            .ok_or_else(|| Status::permission_denied("authenticated user review is required"))?;
        if principal.is_anonymous || principal.kind != crate::auth::PrincipalKind::User {
            return Err(Status::permission_denied(
                "authenticated user review is required",
            ));
        }
        let (visibility, branch) = self.glossary_context(&request).await?;
        let (metadata, extensions, input) = request.into_parts();
        let id = exact(
            input
                .term
                .as_ref()
                .and_then(|reference| reference.id.as_ref()),
        )?
        .clone();
        if input.if_match.is_empty() || input.reason.trim().is_empty() || input.reason.len() > 4096
        {
            return Err(Status::invalid_argument(
                "review requires an etag and a bounded reason",
            ));
        }
        let disposition =
            pb::GlossaryReviewDisposition::try_from(input.disposition).map_err(invalid)?;
        if disposition == pb::GlossaryReviewDisposition::Unspecified {
            return Err(Status::invalid_argument("review disposition is required"));
        }
        let current = self
            .glossary_row(pb::EntityKind::Term, &id, branch.as_deref(), &visibility)
            .await?;
        if current.etag != input.if_match {
            return Err(crate::conv::stale_precondition(
                input.if_match,
                current.etag,
            ));
        }
        let mut entity = current.entity;
        let pool = if term(&entity).is_some_and(|value| value.metadata.is_empty()) {
            TranscodePool::builtin().map_err(invalid)?
        } else {
            self.transcode_pool(&id.namespace, branch.as_deref())
                .await?
        };
        let owner_uid = uid(&entity).to_owned();
        let Some(pb::entity::Kind::Term(value)) = &mut entity.kind else {
            return Err(Status::not_found("term not found"));
        };
        core::validate_term(value).map_err(invalid)?;
        let digest = core::candidate_digest_with_pool(value, &pool).map_err(invalid)?;
        let glossary = value.glossary.as_mut().ok_or_else(|| {
            Status::invalid_argument("generated glossary evidence is required for review")
        })?;
        glossary.review = Some(pb::GlossaryReview {
            disposition: disposition as i32,
            actor: principal.name.to_string(),
            reviewed_at: Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            content_digest: Some(digest),
            owner_uid,
            reason: input.reason,
        });
        let representation = pool
            .entity_json_value(&entity)
            .map_err(|_| Status::invalid_argument("glossary review requires canonical content"))?;
        let bytes = serde_json::to_vec(&representation)
            .map_err(|_| Status::invalid_argument("glossary review cannot be encoded"))?;
        if bytes.len() > 2 * 1024 * 1024 {
            return Err(Status::resource_exhausted(
                "reviewed glossary content exceeds its 2 MiB JSON budget",
            ));
        }
        let permit = entity.clone();
        let inspect = Request::from_parts(metadata.clone(), extensions.clone(), ());
        let put = Request::from_parts(
            metadata,
            extensions,
            pb::PutEntityRequest {
                entity: Some(entity),
                if_match: input.if_match,
                validate_only: input.validate_only,
                ..Default::default()
            },
        );
        let written = REVIEW_PERMIT
            .scope(permit, self.put_entity(put))
            .await?
            .into_inner();
        let context = self.begin_glossary_read(&inspect).await?;
        let row = StoredEntity {
            entity: written
                .entity
                .ok_or_else(|| Status::internal("review write has no entity"))?,
            etag: written.etag,
        };
        let entry = self
            .glossary_entry(
                row,
                context.branch.as_deref(),
                &context.visibility,
                &mut Budget::default(),
            )
            .await;
        let entry = self.finish_glossary_read(&context, entry).await?;
        let mut index = entry.index;
        if input.validate_only {
            index.published = false;
            index.reason = "Review preview was not persisted".into();
        }
        bounded_output_with_pool(
            "GetGlossaryTermResponse",
            pb::GetGlossaryTermResponse {
                entity: Some(entry.stored.entity),
                etag: entry.stored.etag,
                entry: Some(index),
                evidence: entry.evidence,
                incomplete: entry.incomplete,
            },
            &pool,
        )
        .map(Response::new)
    }

    pub(super) async fn glossary_compare(
        &self,
        request: pb::CompareGlossariesRequest,
        branch: Option<&str>,
        visibility: &Visibility,
    ) -> Result<pb::CompareGlossariesResponse, Status> {
        let mut budget = Budget::default();
        let left = self
            .glossary_catalog(
                request
                    .left
                    .ok_or_else(|| Status::invalid_argument("left selection is required"))?,
                branch,
                visibility,
                &mut budget,
            )
            .await?;
        let right = self
            .glossary_catalog(
                request
                    .right
                    .ok_or_else(|| Status::invalid_argument("right selection is required"))?,
                branch,
                visibility,
                &mut budget,
            )
            .await?;
        let left_terms: Vec<_> = left
            .entries
            .iter()
            .filter_map(|entry| term(&entry.stored.entity).cloned())
            .collect();
        let right_terms: Vec<_> = right
            .entries
            .iter()
            .filter_map(|entry| term(&entry.stored.entity).cloned())
            .collect();
        let differences = core::compare_glossaries(&left_terms, &right_terms)
            .map_err(invalid)?
            .into_iter()
            .map(|difference| pb::GlossaryComparisonItem {
                kind: match difference.kind {
                    core::GlossaryDifferenceKind::DifferentMeaning => {
                        pb::GlossaryDifferenceKind::DifferentMeaning
                    }
                    core::GlossaryDifferenceKind::SameMeaning => {
                        pb::GlossaryDifferenceKind::SameMeaning
                    }
                    core::GlossaryDifferenceKind::MissingCounterpart => {
                        pb::GlossaryDifferenceKind::MissingCounterpart
                    }
                } as i32,
                left: difference.left,
                right: difference.right,
                title: difference.title,
                explanation: difference.explanation,
            })
            .collect();
        bounded_output(
            "CompareGlossariesResponse",
            pb::CompareGlossariesResponse {
                differences,
                incomplete: left.incomplete || right.incomplete,
                snapshot_revision: hex(&Sha256::digest(format!(
                    "{}:{}",
                    left.revision, right.revision
                ))),
            },
        )
    }

    pub(super) async fn glossary_check(
        &self,
        request: pb::CheckGlossaryUsageRequest,
        branch: Option<&str>,
        visibility: &Visibility,
        kind: core::UsageTextKind,
    ) -> Result<pb::CheckGlossaryUsageResponse, Status> {
        if request.text_revision.trim().is_empty() || request.text_revision.len() > 1024 {
            return Err(Status::invalid_argument("exact text_revision is required"));
        }
        let selection = request
            .selection
            .ok_or_else(|| Status::invalid_argument("selection is required"))?;
        if selection.include_unreviewed || selection.audience.trim().is_empty() {
            return Err(Status::invalid_argument(
                "usage checks require a published audience selection",
            ));
        }
        let audience = selection.audience.clone();
        let mut budget = Budget::default();
        let catalog = self
            .glossary_catalog(selection, branch, visibility, &mut budget)
            .await?;
        let stale = catalog.entries.iter().any(|entry| {
            !matches!(
                pb::GlossaryEntryState::try_from(entry.index.state),
                Ok(pb::GlossaryEntryState::Reviewed | pb::GlossaryEntryState::ReviewDue)
            )
        });
        if stale || catalog.incomplete {
            return Ok(pb::CheckGlossaryUsageResponse {
                text_revision: request.text_revision,
                snapshot_revision: catalog.revision,
                incomplete: true,
                ..Default::default()
            });
        }
        let at: AuthoredDate = request
            .on
            .as_ref()
            .map_or_else(today, |date| date.value.clone())
            .parse()
            .map_err(invalid)?;
        let ranges = request
            .exclusions
            .iter()
            .map(|range| core::TextRange::new(range.start as usize, range.end as usize))
            .collect::<Result<Vec<_>, _>>()
            .map_err(invalid)?;
        let terms: Vec<_> = catalog
            .entries
            .iter()
            .filter_map(|entry| term(&entry.stored.entity).cloned())
            .collect();
        let mut findings = Vec::new();
        let mut incomplete = false;
        let check = match kind {
            core::UsageTextKind::Prose => core::check_usage,
            core::UsageTextKind::Code => core::check_code_usage,
        };
        for mut finding in check(
            &terms,
            &audience,
            &at,
            &request.text,
            &ranges,
            request.skip_quoted,
        )
        .map_err(invalid)?
        {
            if let Some(preferred) = &finding.preferred_term {
                let target = exact(preferred.id.as_ref())?;
                budget.attempt()?;
                match self
                    .glossary_row(pb::EntityKind::Term, target, branch, visibility)
                    .await
                {
                    Ok(row) => budget.source(&row.entity)?,
                    Err(error) if error.code() == tonic::Code::NotFound => {
                        incomplete = true;
                        finding.preferred_term = None;
                    }
                    Err(error) => return Err(error),
                }
            }
            findings.push(pb::GlossaryUsageFinding {
                term: Some(finding.term),
                range: Some(pb::GlossaryTextRange {
                    start: u32::try_from(finding.range.start()).map_err(invalid)?,
                    end: u32::try_from(finding.range.end()).map_err(invalid)?,
                }),
                matched: finding.matched,
                usage: finding.usage as i32,
                preferred_term: finding.preferred_term,
                rationale: finding.rationale,
            });
        }
        bounded_output(
            "CheckGlossaryUsageResponse",
            pb::CheckGlossaryUsageResponse {
                findings,
                text_revision: request.text_revision,
                snapshot_revision: catalog.revision,
                incomplete,
            },
        )
    }

    pub(super) async fn glossary_refresh(
        &self,
        request: pb::InspectGlossaryRefreshRequest,
        branch: Option<&str>,
        visibility: &Visibility,
    ) -> Result<pb::InspectGlossaryRefreshResponse, Status> {
        if request.generation_revision.trim().is_empty() || request.generation_revision.len() > 1024
        {
            return Err(Status::invalid_argument("generation_revision is required"));
        }
        let mut budget = Budget::default();
        let catalog = self
            .glossary_catalog(
                request
                    .selection
                    .ok_or_else(|| Status::invalid_argument("selection is required"))?,
                branch,
                visibility,
                &mut budget,
            )
            .await?;
        let sources = self
            .glossary_sources_with_budget(
                pb::InspectGlossarySourcesRequest {
                    documents: request.documents,
                },
                branch,
                visibility,
                &mut budget,
            )
            .await?;
        let mut selected_sources = BTreeMap::new();
        for source in &sources.sources {
            let id = exact(
                source
                    .document
                    .as_ref()
                    .and_then(|reference| reference.id.as_ref()),
            )?;
            if selected_sources
                .insert((id.namespace.clone(), id.slug.clone()), id.version)
                .is_some()
            {
                return Err(Status::invalid_argument(
                    "select one exact source version per source family",
                ));
            }
        }
        let observations = catalog
            .entries
            .into_iter()
            .map(|mut entry| {
                if let Some(glossary) = term(&entry.stored.entity).and_then(|value| value.glossary.as_ref()) {
                    for (index, evidence) in glossary.evidence.iter().enumerate() {
                        if let Some(id) = evidence.document.as_ref().and_then(|reference| reference.id.as_ref()) {
                            if selected_sources.get(&(id.namespace.clone(), id.slug.clone())).is_some_and(|version| *version != id.version) {
                                entry.evidence[index].state = pb::GlossaryEvidenceState::Changed as i32;
                                entry.evidence[index].reason = "The selected source revision differs from the reviewed evidence".into();
                                entry.index.state = pb::GlossaryEntryState::SourceChanged as i32;
                                entry.index.reason = "The selected input inventory requires reconciliation".into();
                            }
                        }
                    }
                }
                let generation_changed = term(&entry.stored.entity)
                    .and_then(|term| term.glossary.as_ref())
                    .is_none_or(|value| value.generation_revision != request.generation_revision);
                pb::GlossaryRefreshObservation {
                    entry: Some(entry.index),
                    evidence: entry.evidence,
                    generation_changed,
                }
            })
            .collect();
        bounded_output(
            "InspectGlossaryRefreshResponse",
            pb::InspectGlossaryRefreshResponse {
                observations,
                sources: sources.sources,
                incomplete: catalog.incomplete || sources.incomplete,
                snapshot_revision: hex(&Sha256::digest(format!(
                    "{}:{}:{}",
                    catalog.revision, sources.snapshot_revision, request.generation_revision
                ))),
            },
        )
    }

    async fn glossary_proposed_target(
        &self,
        kind: pb::EntityKind,
        id: &pb::Id,
        metas: &[OpMetaLike],
        branch: Option<&str>,
        visibility: &Visibility,
        landing: bool,
    ) -> Result<StoredEntity, Status> {
        self.require_visible(visibility, &id.namespace).await?;
        if let Some(meta) = metas
            .iter()
            .rev()
            .find(|meta| meta.kind == kind && &meta.id == id)
        {
            let mut entity = meta.entity.clone().filter(|_| meta.is_put).ok_or_else(|| {
                Status::invalid_argument("glossary target is unavailable in the proposed view")
            })?;
            if !landing {
                entity.system = match self.store.get(kind, id, branch).await {
                    Ok(current) => current.entity.system,
                    Err(trogon_atlas_store::error::StoreError::NotFound) => None,
                    Err(error) => return Err(store_err(error)),
                };
            }
            return Ok(StoredEntity {
                entity,
                etag: String::new(),
            });
        }
        self.glossary_row(kind, id, branch, visibility)
            .await
            .map_err(|error| {
                if error.code() == tonic::Code::NotFound {
                    Status::invalid_argument("glossary target is unavailable")
                } else {
                    error
                }
            })
    }

    async fn validate_glossary_library_changes(
        &self,
        metas: &[OpMetaLike],
        changes: &crate::type_libraries::TypeLibraryChanges,
        branch: Option<&str>,
        budget: &mut Budget,
    ) -> Result<(), Status> {
        let namespaces: Vec<_> = metas
            .iter()
            .filter(|meta| meta.kind == pb::EntityKind::TypeLibrary)
            .map(|meta| meta.id.namespace.clone())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        if namespaces.is_empty() {
            return Ok(());
        }
        let rows = self
            .store
            .list_bounded(
                ListFilter {
                    kinds: &[pb::EntityKind::Term],
                    namespaces: &namespaces,
                    ..Default::default()
                },
                Some(MAX_ROWS + 1),
                branch,
                trogon_atlas_store::EntityByteBudget::new(MAX_CATALOG_BYTES),
            )
            .await
            .map_err(store_err)?;
        if rows.len() > MAX_ROWS {
            return Err(Status::resource_exhausted(
                "glossary descriptor users exceed their row budget",
            ));
        }
        let mut proposed = HashMap::new();
        for row in rows {
            let key = pb::EntityKey::new(pb::EntityKind::Term, super::entity_id(&row.entity)?);
            proposed.insert(key, row.entity);
        }
        for meta in metas.iter().filter(|meta| {
            meta.kind == pb::EntityKind::Term && namespaces.contains(&meta.id.namespace)
        }) {
            let key = pb::EntityKey::new(meta.kind, &meta.id);
            if let Some(entity) = meta.entity.as_ref().filter(|_| meta.is_put) {
                proposed.insert(key, entity.clone());
            } else {
                proposed.remove(&key);
            }
        }
        if proposed.len() > MAX_ROWS
            || proposed.values().map(Message::encoded_len).sum::<usize>() > MAX_CATALOG_BYTES
        {
            return Err(Status::resource_exhausted(
                "glossary descriptor users exceed their proposal budget",
            ));
        }
        for value in proposed.values().filter_map(term).filter(|term| {
            !term.metadata.is_empty()
                && term
                    .glossary
                    .as_ref()
                    .and_then(|entry| entry.review.as_ref())
                    .is_some_and(|review| {
                        review.disposition == pb::GlossaryReviewDisposition::Accepted as i32
                    })
        }) {
            let namespace = &exact(value.id.as_ref())?.namespace;
            let pool = self
                .glossary_pool(namespace, changes, branch, budget)
                .await?;
            core::candidate_digest_with_pool(value, &pool).map_err(|_| Status::failed_precondition("type library change would strand accepted glossary metadata; review its retirement first"))?;
        }
        Ok(())
    }

    pub(super) async fn validate_glossary_writes(
        &self,
        metas: &[OpMetaLike],
        branch: Option<&str>,
        visibility: &Visibility,
        landing: Option<&[BranchLandOp]>,
    ) -> Result<(), Status> {
        let mut budget = Budget::default();
        let mut library_changes = crate::type_libraries::TypeLibraryChanges::default();
        for meta in metas {
            if let Some(entity) = meta.entity.as_ref().filter(|_| meta.is_put) {
                library_changes.put(entity);
            } else {
                library_changes.delete(meta.kind, &meta.id);
            }
        }
        self.validate_glossary_library_changes(metas, &library_changes, branch, &mut budget)
            .await?;
        for meta in metas
            .iter()
            .filter(|meta| meta.kind == pb::EntityKind::Term)
        {
            let previous = match self.store.get(meta.kind, &meta.id, branch).await {
                Ok(row) => Some(row.entity),
                Err(trogon_atlas_store::error::StoreError::NotFound) => None,
                Err(error) => return Err(store_err(error)),
            };
            let old = previous.as_ref().and_then(term);
            let old_review = old
                .and_then(|term| term.glossary.as_ref())
                .and_then(|value| value.review.as_ref());
            if !meta.is_put {
                if old_review.is_some_and(|review| {
                    review.disposition == pb::GlossaryReviewDisposition::Accepted as i32
                }) {
                    return Err(Status::failed_precondition(
                        "review the glossary retirement before deleting a published version",
                    ));
                }
                continue;
            }
            let proposed = meta
                .entity
                .as_ref()
                .ok_or_else(|| Status::invalid_argument("term is required"))?;
            let value =
                term(proposed).ok_or_else(|| Status::invalid_argument("term kind is required"))?;
            core::validate_term(value).map_err(invalid)?;
            let new_review = value
                .glossary
                .as_ref()
                .and_then(|value| value.review.as_ref());
            let permit = REVIEW_PERMIT
                .try_with(|authorized| authorized == proposed)
                .unwrap_or(false);
            let needs_authority = permit
                || (landing.is_some() && new_review.is_some())
                || old_review.is_some_and(|review| {
                    review.disposition == pb::GlossaryReviewDisposition::Accepted as i32
                });
            let pool = if needs_authority
                && (!value.metadata.is_empty()
                    || old.is_some_and(|value| !value.metadata.is_empty()))
            {
                self.glossary_pool(&meta.id.namespace, &library_changes, branch, &mut budget)
                    .await?
            } else {
                TranscodePool::builtin().map_err(invalid)?
            };
            if permit
                && !previous.as_ref().is_some_and(|current| {
                    core::review_is_current_with_pool(value, uid(current), &pool)
                })
            {
                return Err(Status::failed_precondition(
                    "glossary review descriptors changed before admission; retry",
                ));
            }
            let trusted_landing = landing.is_some()
                && new_review.is_some()
                && core::review_is_current_with_pool(value, uid(proposed), &pool);
            if old_review != new_review && !permit && !trusted_landing {
                return Err(Status::invalid_argument(
                    "glossary reviews are server-owned; use ReviewGlossaryTerm",
                ));
            }
            if let Some(old) = old.filter(|_| {
                old_review.is_some_and(|review| {
                    review.disposition == pb::GlossaryReviewDisposition::Accepted as i32
                })
            }) {
                if core::candidate_digest_with_pool(old, &pool).map_err(invalid)?
                    != core::candidate_digest_with_pool(value, &pool).map_err(invalid)?
                {
                    return Err(Status::failed_precondition(
                        "published glossary meaning is immutable; create a new version",
                    ));
                }
            }
            if (permit || trusted_landing)
                && new_review.is_some_and(|review| {
                    matches!(
                        pb::GlossaryReviewDisposition::try_from(review.disposition),
                        Ok(pb::GlossaryReviewDisposition::Rejected
                            | pb::GlossaryReviewDisposition::NeedsChanges)
                    )
                })
            {
                let current_digest = old
                    .map(|value| core::candidate_digest_with_pool(value, &pool))
                    .transpose()
                    .map_err(invalid)?;
                if previous.as_ref().is_some_and(|current| {
                    core::review_is_current_with_pool(value, uid(current), &pool)
                }) && current_digest.as_ref()
                    == new_review.and_then(|review| review.content_digest.as_ref())
                {
                    continue;
                }
                if permit {
                    return Err(Status::failed_precondition(
                        "negative review requires unchanged current glossary content",
                    ));
                }
            }
            let Some(glossary) = &value.glossary else {
                continue;
            };
            for evidence in &glossary.evidence {
                let unchanged = old
                    .and_then(|term| term.glossary.as_ref())
                    .is_some_and(|old| old.evidence.contains(evidence));
                if unchanged && !permit && !trusted_landing {
                    continue;
                }
                let id = exact(
                    evidence
                        .document
                        .as_ref()
                        .and_then(|reference| reference.id.as_ref()),
                )?;
                budget.attempt()?;
                let row = self
                    .glossary_proposed_target(
                        pb::EntityKind::Document,
                        id,
                        metas,
                        branch,
                        visibility,
                        landing.is_some(),
                    )
                    .await?;
                budget.source(&row.entity)?;
                core::verify_evidence(evidence, &row.entity).map_err(invalid)?;
            }
            let mut current = glossary
                .policy
                .as_ref()
                .and_then(|policy| policy.preferred_term.clone());
            let mut visited = HashSet::new();
            visited.insert(pb::EntityKey::new(pb::EntityKind::Term, &meta.id));
            while let Some(reference) = current {
                let id = exact(reference.id.as_ref())?;
                if visited.len() > 1000 {
                    return Err(Status::resource_exhausted(
                        "glossary replacement traversal exceeds its budget",
                    ));
                }
                if !visited.insert(pb::EntityKey::new(pb::EntityKind::Term, id)) {
                    return Err(Status::invalid_argument("glossary preferred-term cycle"));
                }
                budget.attempt()?;
                let row = self
                    .glossary_proposed_target(
                        pb::EntityKind::Term,
                        id,
                        metas,
                        branch,
                        visibility,
                        landing.is_some(),
                    )
                    .await?;
                budget.source(&row.entity)?;
                let target = term(&row.entity)
                    .ok_or_else(|| Status::invalid_argument("preferred term is unavailable"))?;
                current = target
                    .glossary
                    .as_ref()
                    .and_then(|entry| entry.policy.as_ref())
                    .and_then(|policy| policy.preferred_term.clone());
            }
        }
        Ok(())
    }
}
