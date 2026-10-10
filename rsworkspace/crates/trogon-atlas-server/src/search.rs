//! Tantivy-backed search over `StoredEntity` snapshots.
//!
//! [`SearchIndex`] is a long-lived in-RAM index. It is seeded once from a
//! full store snapshot ([`SearchIndex::replace_all`]) and then kept in
//! lockstep with store writes via [`SearchIndex::apply_put`] /
//! [`SearchIndex::apply_delete`], so searches never pay an O(N) rebuild.

use std::collections::HashMap;

use parking_lot::{Mutex, RwLock};
use tantivy::{
    collector::TopDocs,
    doc,
    query::{BooleanQuery, Occur, Query, QueryParser, TermQuery},
    schema::{Field, IndexRecordOption, Schema, Value, STORED, STRING, TEXT},
    Index, IndexReader, IndexWriter, TantivyDocument, Term,
};
use trogon_atlas_proto as pb;
use trogon_atlas_store::store::StoredEntity;

pub struct SearchIndex {
    index: Index,
    fields: SearchFields,
    writer: Mutex<IndexWriter>,
    reader: IndexReader,
    /// Owner of the original Entity values keyed by document id: Tantivy
    /// only stores the strings we feed it, so we keep the structured form
    /// here to return alongside the score.
    entities: RwLock<HashMap<String, pb::Entity>>,
}

struct SearchFields {
    doc_id: Field,
    kind: Field,
    namespace: Field,
    slug: Field,
    title: Field,
    body: Field,
}

fn build_schema() -> (Schema, SearchFields) {
    let mut builder = Schema::builder();
    let doc_id = builder.add_text_field("doc_id", STRING | STORED);
    // Exact-match (untokenized), not TEXT: `kind`/`namespace` are filter
    // values, never parsed as free text, and several kind labels contain
    // underscores (`read_model`) that a tokenizer would split into terms a
    // `TermQuery` could then confuse with an unrelated label (`event_model`).
    let kind = builder.add_text_field("kind", STRING | STORED);
    let namespace = builder.add_text_field("namespace", STRING | STORED);
    let slug = builder.add_text_field("slug", TEXT | STORED);
    let title = builder.add_text_field("title", TEXT | STORED);
    let body = builder.add_text_field("body", TEXT);
    let schema = builder.build();
    let fields = SearchFields {
        doc_id,
        kind,
        namespace,
        slug,
        title,
        body,
    };
    (schema, fields)
}

fn doc_key(kind: pb::EntityKind, id: &pb::Id) -> String {
    format!(
        "{}/{}/{}@{}",
        kind_label_for(kind),
        id.namespace,
        id.slug,
        id.version
    )
}

fn entity_doc_key(entity: &pb::Entity) -> Option<String> {
    let kind = trogon_atlas_store::refs::entity_kind(entity)?;
    let id = trogon_atlas_store::refs::entity_id(entity)?;
    Some(doc_key(kind, id))
}

/// Pre-warm `search_index` from `store` so the first `SearchEntities` RPC
/// does not pay the full `list + replace_all` cost. Spawned by `main.rs`
/// at server boot; the lazy path in `EventModelServiceImpl::ensure_search_index`
/// remains as a fallback if this fails.
pub fn spawn_warmup(
    store: std::sync::Arc<dyn trogon_atlas_store::Store>,
    search_index: std::sync::Arc<SearchIndex>,
    search_ready: std::sync::Arc<std::sync::atomic::AtomicBool>,
    entity_cap: usize,
) {
    use std::sync::atomic::Ordering;
    tokio::spawn(async move {
        if search_ready.load(Ordering::Acquire) {
            return;
        }
        let all = match store
            .list(
                trogon_atlas_store::store::ListFilter::default(),
                Some(entity_cap),
                None,
            )
            .await
        {
            Ok(v) => v,
            Err(err) => {
                tracing::warn!(error = %err, "search warmup: store list failed");
                return;
            }
        };
        match search_index.replace_all(&all) {
            Ok(()) => {
                search_ready.store(true, Ordering::Release);
                tracing::info!(entities = all.len(), "search index warmup complete");
            }
            Err(err) => {
                tracing::warn!(error = %err, "search warmup: replace_all failed");
            }
        }
    });
}

impl SearchIndex {
    pub fn new() -> tantivy::Result<Self> {
        let (schema, fields) = build_schema();
        let index = Index::create_in_ram(schema);
        let writer = index.writer_with_num_threads(1, 15_000_000)?;
        let reader = index.reader()?;
        Ok(SearchIndex {
            index,
            fields,
            writer: Mutex::new(writer),
            reader,
            entities: RwLock::new(HashMap::new()),
        })
    }

    /// Rebuild the whole index from a store snapshot.
    pub fn replace_all(&self, entities: &[StoredEntity]) -> tantivy::Result<()> {
        let mut owned: HashMap<String, pb::Entity> = HashMap::with_capacity(entities.len());
        {
            let mut writer = self.writer.lock();
            writer.delete_all_documents()?;
            for se in entities {
                let Some(key) = entity_doc_key(&se.entity) else {
                    continue;
                };
                self.add_doc(&mut writer, &key, &se.entity)?;
                owned.insert(key, se.entity.clone());
            }
            writer.commit()?;
        }
        *self.entities.write() = owned;
        self.reader.reload()
    }

    /// Upsert one entity into the live index.
    pub fn apply_put(&self, entity: &pb::Entity) -> tantivy::Result<()> {
        let Some(key) = entity_doc_key(entity) else {
            return Ok(());
        };
        {
            let mut writer = self.writer.lock();
            writer.delete_term(Term::from_field_text(self.fields.doc_id, &key));
            self.add_doc(&mut writer, &key, entity)?;
            writer.commit()?;
        }
        self.entities.write().insert(key, entity.clone());
        self.reader.reload()
    }

    /// Remove one entity from the live index.
    pub fn apply_delete(&self, kind: pb::EntityKind, id: &pb::Id) -> tantivy::Result<()> {
        let key = doc_key(kind, id);
        {
            let mut writer = self.writer.lock();
            writer.delete_term(Term::from_field_text(self.fields.doc_id, &key));
            writer.commit()?;
        }
        self.entities.write().remove(&key);
        self.reader.reload()
    }

    fn add_doc(
        &self,
        writer: &mut IndexWriter,
        key: &str,
        entity: &pb::Entity,
    ) -> tantivy::Result<()> {
        let kind = kind_label(entity);
        let (namespace, slug) = id_parts(entity);
        let title = title_of(entity);
        let body = body_of(entity);
        writer.add_document(doc!(
            self.fields.doc_id => key.to_string(),
            self.fields.kind => kind,
            self.fields.namespace => namespace,
            self.fields.slug => slug,
            self.fields.title => title,
            self.fields.body => body,
        ))?;
        Ok(())
    }

    /// Run a free-text query. `kind_filter` and `namespace_filter`, when
    /// non-empty, restrict results to those exact values. Returns
    /// `(entity, score, excerpt)` triples sorted by descending score.
    pub fn search(
        &self,
        query: &str,
        kind_filter: &[String],
        namespace_filter: &[String],
        limit: usize,
    ) -> tantivy::Result<Vec<(pb::Entity, f32, String)>> {
        let searcher = self.reader.searcher();
        let parser = QueryParser::for_index(
            &self.index,
            vec![self.fields.title, self.fields.body, self.fields.slug],
        );
        // The user's free-text query is escaped to a sequence of plain
        // terms: no Tantivy syntax (field-prefix, boolean ops, ranges,
        // wildcards) survives. This prevents a query like
        // `namespace:other-ns text` from steering the searcher around the
        // `kind_filter` / `namespace_filter` we apply post-search.
        let escaped = escape_query(query);
        if escaped.trim().is_empty() {
            return Ok(Vec::new());
        }
        let parsed = match parser.parse_query(&escaped) {
            Ok(q) => q,
            Err(e) => {
                // A parse failure after escaping indicates a query the model
                // cannot handle even after stripping special characters. Log
                // it so operators can observe the pattern, then surface it as
                // an error -- the caller can convert to the appropriate gRPC
                // status (InvalidArgument). Silently returning empty results
                // would make unparseable queries indistinguishable from "no
                // matches" from the client's perspective.
                let snippet: String = escaped.chars().take(200).collect();
                tracing::warn!(
                    query = %snippet,
                    error = %e,
                    "tantivy failed to parse search query"
                );
                return Err(tantivy::TantivyError::InvalidArgument(format!(
                    "unparseable search query: {e}"
                )));
            }
        };

        // `kind_filter`/`namespace_filter` are folded into the query itself
        // (as exact-match `TermQuery` clauses) rather than applied to the
        // results after the fact. A post-hoc filter has to over-fetch to
        // guess how many of the top-scored docs will survive it, and an
        // unlucky corpus (many off-filter docs outscoring the handful that
        // match) starves the caller of a full page despite enough real
        // matches existing further down the ranking. Filtering inside the
        // query means `TopDocs` ranks only among docs that already pass the
        // filter, so the requested `limit` is only ever short of matches
        // that actually don't exist.
        let safe_limit = limit.clamp(1, 1000);
        let full_query = Self::filtered_query(parsed, &self.fields, kind_filter, namespace_filter);
        let top_docs = searcher.search(
            &full_query,
            &TopDocs::with_limit(safe_limit).order_by_score(),
        )?;

        struct Match {
            doc_id: String,
            score: f32,
            excerpt: String,
        }
        let mut candidates: Vec<Match> = Vec::with_capacity(top_docs.len());
        for (score, addr) in top_docs {
            let doc: TantivyDocument = searcher.doc(addr)?;
            let doc_id = first_text(&doc, self.fields.doc_id).unwrap_or_default();
            let excerpt = excerpt_for(&doc, &self.fields, query);
            candidates.push(Match {
                doc_id,
                score,
                excerpt,
            });
        }

        let entities = self.entities.read();
        let out = candidates
            .into_iter()
            .filter_map(|m| {
                let entity = entities.get(&m.doc_id)?.clone();
                Some((entity, m.score, m.excerpt))
            })
            .collect();
        Ok(out)
    }

    /// AND `parsed` with exact-match disjunctions over `kind_filter` and
    /// `namespace_filter` (each empty list is "no restriction"), so a filter
    /// narrows which documents `TopDocs` ranks among, not which of the
    /// already-ranked top docs get kept.
    fn filtered_query(
        parsed: Box<dyn Query>,
        fields: &SearchFields,
        kind_filter: &[String],
        namespace_filter: &[String],
    ) -> Box<dyn Query> {
        fn exact_match_any(field: Field, values: &[String]) -> Box<dyn Query> {
            let clauses = values
                .iter()
                .map(|v| {
                    let term =
                        TermQuery::new(Term::from_field_text(field, v), IndexRecordOption::Basic);
                    (Occur::Should, Box::new(term) as Box<dyn Query>)
                })
                .collect();
            Box::new(BooleanQuery::new(clauses))
        }

        let mut clauses: Vec<(Occur, Box<dyn Query>)> = vec![(Occur::Must, parsed)];
        if !kind_filter.is_empty() {
            clauses.push((Occur::Must, exact_match_any(fields.kind, kind_filter)));
        }
        if !namespace_filter.is_empty() {
            clauses.push((
                Occur::Must,
                exact_match_any(fields.namespace, namespace_filter),
            ));
        }
        Box::new(BooleanQuery::new(clauses))
    }
}

/// Escape a user-supplied query so Tantivy treats it as a sequence of plain
/// terms. Any character that has special meaning in Tantivy's query syntax
/// (`+ - && || ! ( ) { } [ ] ^ " ~ * ? : \ /`) is replaced with a single
/// space; consecutive whitespace collapses to one. Field-prefixed terms
/// (`title:foo`) become `title foo`, which the `QueryParser` then searches
/// against all default fields as text.
fn escape_query(query: &str) -> String {
    // `-` is intentionally NOT in this list: entity slugs commonly carry
    // hyphens (`order-placed`, `place-order-slice`) and a search for one
    // must still match. Tantivy's MUST_NOT operator is a leading `-` on
    // a term; we mitigate the steering risk by also stripping `+`, which
    // is the matching MUST operator, so the only escapable boolean
    // primitives left are the bare words "AND" / "OR" / "NOT".
    const SPECIAL: &[char] = &[
        '+', '!', '(', ')', '{', '}', '[', ']', '^', '"', '~', '*', '?', ':', '\\', '/', '&', '|',
    ];
    let mut out = String::with_capacity(query.len());
    let mut last_was_space = true;
    for ch in query.chars() {
        let mapped = if SPECIAL.contains(&ch) || ch.is_control() {
            ' '
        } else {
            ch
        };
        if mapped == ' ' {
            if !last_was_space {
                out.push(' ');
                last_was_space = true;
            }
        } else {
            out.push(mapped);
            last_was_space = false;
        }
    }
    out.trim().to_string()
}

fn first_text(doc: &TantivyDocument, field: Field) -> Option<String> {
    doc.get_first(field)
        .and_then(|v| v.as_str())
        .map(std::borrow::ToOwned::to_owned)
}

fn excerpt_for(doc: &TantivyDocument, fields: &SearchFields, query: &str) -> String {
    let title = first_text(doc, fields.title).unwrap_or_default();
    if !title.is_empty() {
        return title;
    }
    let term = query
        .split_whitespace()
        .next()
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    let slug = first_text(doc, fields.slug).unwrap_or_default();
    if slug.to_ascii_lowercase().contains(&term) {
        return slug;
    }
    String::new()
}

fn kind_label(entity: &pb::Entity) -> String {
    use pb::entity::Kind as K;
    match entity.kind.as_ref() {
        Some(K::Event(_)) => "event",
        Some(K::Command(_)) => "command",
        Some(K::ReadModel(_)) => "read_model",
        Some(K::Processor(_)) => "processor",
        Some(K::Ui(_)) => "ui",
        Some(K::Persona(_)) => "persona",
        Some(K::Swimlane(_)) => "swimlane",
        Some(K::CommandSlice(_)) => "command_slice",
        Some(K::ReadModelSlice(_)) => "read_model_slice",
        Some(K::AutomationSlice(_)) => "automation_slice",
        Some(K::UiSlice(_)) => "ui_slice",
        Some(K::Storyboard(_)) => "storyboard",
        Some(K::EventModel(_)) => "event_model",
        Some(K::Component(_)) => "component",
        Some(K::ExternalSystem(_)) => "external_system",
        Some(K::Tracker(_)) => "tracker",
        Some(K::BoundedContext(_)) => "bounded_context",
        Some(K::Domain(_)) => "domain",
        Some(K::Subdomain(_)) => "subdomain",
        Some(K::Schema(_)) => "schema",
        Some(K::Project(_)) => "project",
        Some(K::Screen(_)) => "screen",
        Some(K::Term(_)) => "term",
        Some(K::Ambiguity(_)) => "ambiguity",
        Some(K::ServiceLevelIndicator(_)) => "service_level_indicator",
        Some(K::ServiceLevelObjective(_)) => "service_level_objective",
        Some(K::AlertPolicy(_)) => "alert_policy",
        Some(K::AlertNotificationTarget(_)) => "alert_notification_target",
        Some(K::TypeLibrary(_)) => "type_library",
        None => "",
    }
    .to_string()
}

#[must_use]
pub fn kind_label_for(kind: pb::EntityKind) -> &'static str {
    match kind {
        pb::EntityKind::Event => "event",
        pb::EntityKind::Command => "command",
        pb::EntityKind::ReadModel => "read_model",
        pb::EntityKind::Processor => "processor",
        pb::EntityKind::Ui => "ui",
        pb::EntityKind::Persona => "persona",
        pb::EntityKind::Swimlane => "swimlane",
        pb::EntityKind::CommandSlice => "command_slice",
        pb::EntityKind::ReadModelSlice => "read_model_slice",
        pb::EntityKind::AutomationSlice => "automation_slice",
        pb::EntityKind::UiSlice => "ui_slice",
        pb::EntityKind::Storyboard => "storyboard",
        pb::EntityKind::EventModel => "event_model",
        pb::EntityKind::Component => "component",
        pb::EntityKind::ExternalSystem => "external_system",
        pb::EntityKind::Tracker => "tracker",
        pb::EntityKind::BoundedContext => "bounded_context",
        pb::EntityKind::Domain => "domain",
        pb::EntityKind::Subdomain => "subdomain",
        pb::EntityKind::Schema => "schema",
        pb::EntityKind::Project => "project",
        pb::EntityKind::Screen => "screen",
        pb::EntityKind::Term => "term",
        pb::EntityKind::Ambiguity => "ambiguity",
        pb::EntityKind::ServiceLevelIndicator => "service_level_indicator",
        pb::EntityKind::ServiceLevelObjective => "service_level_objective",
        pb::EntityKind::AlertPolicy => "alert_policy",
        pb::EntityKind::AlertNotificationTarget => "alert_notification_target",
        pb::EntityKind::TypeLibrary => "type_library",
        pb::EntityKind::Unspecified => "",
    }
}

fn id_parts(entity: &pb::Entity) -> (String, String) {
    trogon_atlas_store::refs::entity_id(entity)
        .map(|id| (id.namespace.clone(), id.slug.clone()))
        .unwrap_or_default()
}

fn title_of(entity: &pb::Entity) -> String {
    use pb::entity::Kind as K;
    match entity.kind.as_ref() {
        Some(K::Event(x)) => x.title.clone(),
        Some(K::Command(x)) => x.title.clone(),
        Some(K::ReadModel(x)) => x.title.clone(),
        Some(K::Processor(x)) => x.title.clone(),
        Some(K::Ui(x)) => x.title.clone(),
        Some(K::Persona(x)) => x.title.clone(),
        Some(K::Swimlane(x)) => x.title.clone(),
        Some(K::CommandSlice(x)) => x.title.clone(),
        Some(K::ReadModelSlice(x)) => x.title.clone(),
        Some(K::AutomationSlice(x)) => x.title.clone(),
        Some(K::UiSlice(x)) => x.title.clone(),
        Some(K::Storyboard(x)) => x.title.clone(),
        Some(K::EventModel(x)) => x.title.clone(),
        Some(K::Component(x)) => x.title.clone(),
        Some(K::ExternalSystem(x)) => x.title.clone(),
        Some(K::Tracker(x)) => x.title.clone(),
        Some(K::BoundedContext(x)) => x.title.clone(),
        Some(K::Domain(x)) => x.title.clone(),
        Some(K::Subdomain(x)) => x.title.clone(),
        Some(K::Schema(x)) => x.title.clone(),
        Some(K::Project(x)) => x.title.clone(),
        Some(K::Screen(x)) => x.title.clone(),
        Some(K::Term(x)) => x.title.clone(),
        Some(K::Ambiguity(x)) => x.id.as_ref().map(|id| id.slug.clone()).unwrap_or_default(),
        Some(K::ServiceLevelIndicator(x)) => x.title.clone(),
        Some(K::ServiceLevelObjective(x)) => x.title.clone(),
        Some(K::AlertPolicy(x)) => x.title.clone(),
        Some(K::AlertNotificationTarget(x)) => x.title.clone(),
        Some(K::TypeLibrary(x)) => x.title.clone(),
        None => String::new(),
    }
}

fn body_of(entity: &pb::Entity) -> String {
    use pb::entity::Kind as K;
    let mut s = String::new();
    let push = |s: &mut String, x: &str| {
        if !x.is_empty() {
            s.push_str(x);
            s.push('\n');
        }
    };
    match entity.kind.as_ref() {
        Some(K::Event(x)) => {
            push(&mut s, &x.doc);
            for f in &trogon_atlas_core::schema::schema_fields(x.schema.as_ref()) {
                push(&mut s, &f.name);
                push(&mut s, &f.doc);
            }
        }
        Some(K::Command(x)) => {
            push(&mut s, &x.doc);
            for f in &trogon_atlas_core::schema::schema_fields(x.schema.as_ref()) {
                push(&mut s, &f.name);
                push(&mut s, &f.doc);
            }
        }
        Some(K::ReadModel(x)) => {
            push(&mut s, &x.doc);
            for f in &trogon_atlas_core::schema::schema_fields(x.schema.as_ref()) {
                push(&mut s, &f.name);
                push(&mut s, &f.doc);
            }
        }
        Some(K::Processor(x)) => push(&mut s, &x.doc),
        Some(K::Ui(x)) => push(&mut s, &x.doc),
        Some(K::Persona(x)) => push(&mut s, &x.doc),
        Some(K::Swimlane(x)) => push(&mut s, &x.doc),
        Some(K::CommandSlice(x)) => push(&mut s, &x.doc),
        Some(K::ReadModelSlice(x)) => push(&mut s, &x.doc),
        Some(K::AutomationSlice(x)) => push(&mut s, &x.doc),
        Some(K::UiSlice(x)) => push(&mut s, &x.doc),
        Some(K::Storyboard(x)) => push(&mut s, &x.doc),
        Some(K::EventModel(x)) => push(&mut s, &x.doc),
        Some(K::Component(x)) => push(&mut s, &x.doc),
        Some(K::ExternalSystem(x)) => push(&mut s, &x.doc),
        Some(K::Tracker(x)) => push(&mut s, &x.doc),
        Some(K::BoundedContext(x)) => push(&mut s, &x.doc),
        Some(K::Domain(x)) => push(&mut s, &x.doc),
        Some(K::Subdomain(x)) => push(&mut s, &x.doc),
        Some(K::Schema(x)) => push(&mut s, &x.doc),
        Some(K::Project(x)) => push(&mut s, &x.doc),
        Some(K::Screen(x)) => {
            push(&mut s, &x.doc);
            push(&mut s, &x.route);
            push(&mut s, &x.audience);
            push(&mut s, &x.layout);
            for slot in &x.slots {
                push(&mut s, &slot.name);
                push(&mut s, &slot.doc);
            }
        }
        Some(K::Term(x)) => push(&mut s, &x.doc),
        Some(K::Ambiguity(x)) => push(&mut s, &x.doc),
        Some(K::ServiceLevelIndicator(x)) => push(&mut s, &x.doc),
        Some(K::ServiceLevelObjective(x)) => push(&mut s, &x.doc),
        Some(K::AlertPolicy(x)) => {
            push(&mut s, &x.doc);
            push(&mut s, &x.runbook);
        }
        Some(K::AlertNotificationTarget(x)) => {
            push(&mut s, &x.doc);
            push(&mut s, &x.target);
        }
        Some(K::TypeLibrary(x)) => {
            push(&mut s, &x.doc);
            for f in &x.files {
                push(&mut s, &f.path);
            }
        }
        None => {}
    }
    s
}

// ---------------------------------------------------------------------------
// Branch view identity and the one-shot index cache
// ---------------------------------------------------------------------------

/// Domain tag, so a view version can never be confused with an entity or
/// snapshot content hash even though all three are SHA-256.
const VIEW_DOMAIN: &[u8] = b"trogonatlas.search.view.v1\0";

/// How many branch indexes stay resident.
///
/// Each entry is a full in-RAM index over that branch's *merged* view, so
/// the cache trades memory against rebuilds at roughly the cost of the
/// baseline index per branch. Four covers a review session moving between a
/// handful of branches without letting an unbounded number of short-lived
/// branches multiply the server's footprint.
const MAX_CACHED_BRANCH_INDEXES: usize = 4;

/// Identity of one merged branch view: what a cached index was built from.
///
/// Deliberately *not* a snapshot id. `GetSnapshotId` answers "is this the
/// same model", which requires canonical JSON per entity and is far too
/// expensive to compute on every query. A cache key only has to answer "is
/// this the same set of stored revisions", and the store already versions
/// every entity with an etag, so this hashes `(doc key, etag)` pairs. Two
/// views with the same revisions are the same view; a view that differs by
/// one write differs by that entity's etag.
///
/// Entities the index would skip (no resolvable kind or id) are skipped here
/// too. They cannot affect the index, so they must not affect its version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewVersion([u8; 32]);

impl ViewVersion {
    #[must_use]
    pub fn of(entities: &[StoredEntity]) -> Self {
        use sha2::{Digest as _, Sha256};

        // Sorted, because the store's enumeration order is not part of the
        // view's identity and two scans that return the same rows in a
        // different order must produce the same version.
        let mut rows: Vec<(String, &str)> = entities
            .iter()
            .filter_map(|se| Some((entity_doc_key(&se.entity)?, se.etag.as_str())))
            .collect();
        rows.sort_unstable();

        let mut hasher = Sha256::new();
        hasher.update(VIEW_DOMAIN);
        hasher.update(rows.len().to_string().as_bytes());
        hasher.update(b"\0");
        for (key, etag) in &rows {
            // Length-prefixed, so `("ab", "c")` and `("a", "bc")` cannot
            // hash alike.
            hasher.update(key.len().to_string().as_bytes());
            hasher.update(b":");
            hasher.update(key.as_bytes());
            hasher.update(etag.len().to_string().as_bytes());
            hasher.update(b":");
            hasher.update((*etag).as_bytes());
        }
        Self(hasher.finalize().into())
    }
}

struct CachedBranchIndex {
    branch: String,
    version: ViewVersion,
    index: std::sync::Arc<SearchIndex>,
}

/// Keeps the one-shot indexes that branch searches build, so a branch that
/// has not changed is not re-indexed on every query.
///
/// Branch writes are deliberately absent from the live baseline index (see
/// `docs/explanation/branching.md`), so a branch query has to search
/// something built from the merged view. Building that per request makes
/// branch search O(store) while baseline search is O(query) -- the cost this
/// cache removes.
///
/// Correctness rests entirely on [`ViewVersion`]: an entry is served only
/// when the view it was built from hashes identically to the view in hand.
/// Nothing here has to be invalidated by writers, which matters because
/// branch writes intentionally do not touch the baseline cache-invalidation
/// path.
///
/// One entry per branch. A branch's view moves forward, so an older version
/// of the same branch is dead weight rather than a second useful entry.
pub struct BranchIndexCache {
    entries: Mutex<Vec<CachedBranchIndex>>,
    capacity: usize,
}

impl Default for BranchIndexCache {
    fn default() -> Self {
        Self::new(MAX_CACHED_BRANCH_INDEXES)
    }
}

impl BranchIndexCache {
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            entries: Mutex::new(Vec::new()),
            capacity: capacity.max(1),
        }
    }

    /// The cached index for `branch` when it was built from exactly this
    /// view, promoted to most-recently-used.
    #[must_use]
    pub fn get(&self, branch: &str, version: &ViewVersion) -> Option<std::sync::Arc<SearchIndex>> {
        let mut entries = self.entries.lock();
        let position = entries
            .iter()
            .position(|e| e.branch == branch && e.version == *version)?;
        let entry = entries.remove(position);
        let index = entry.index.clone();
        entries.push(entry);
        Some(index)
    }

    /// Store `index` as the cached build for `branch` at `version`,
    /// replacing whatever that branch had before and evicting the
    /// least-recently-used branch if the cache is now over capacity.
    pub fn insert(&self, branch: String, version: ViewVersion, index: std::sync::Arc<SearchIndex>) {
        let mut entries = self.entries.lock();
        entries.retain(|e| e.branch != branch);
        entries.push(CachedBranchIndex {
            branch,
            version,
            index,
        });
        while entries.len() > self.capacity {
            entries.remove(0);
        }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.lock().len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod escape_tests {
    use super::escape_query;

    #[test]
    fn strips_tantivy_metacharacters() {
        for (input, expected) in [
            ("title:foo", "title foo"),
            ("namespace:other-ns text", "namespace other-ns text"),
            ("foo OR bar", "foo OR bar"),
            ("foo*bar?baz", "foo bar baz"),
            ("[* TO *]", "TO"),
            ("nested(parens)", "nested parens"),
            (r#""quoted""#, "quoted"),
            ("\nleading\tcontrol", "leading control"),
        ] {
            let got = escape_query(input);
            assert_eq!(got, expected, "escape_query({input:?}) -> {got:?}");
        }
    }

    #[test]
    fn preserves_hyphenated_slugs() {
        // Slugs like `order-placed` are routine; the user must still be
        // able to type one as a search term and match.
        assert_eq!(escape_query("order-placed"), "order-placed");
    }

    #[test]
    fn empty_after_escape_is_empty_string() {
        assert_eq!(escape_query("*?:[]"), "");
        assert_eq!(escape_query("   "), "");
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use trogon_atlas_proto as pb;

    use super::*;

    fn ev(namespace: &str, slug: &str, title: &str, doc: &str) -> StoredEntity {
        let id = pb::Id {
            namespace: namespace.into(),
            slug: slug.into(),
            version: 0,
        };
        let entity = pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Event(pb::Event {
                id: Some(id),
                title: title.into(),
                doc: doc.into(),
                ..Default::default()
            })),
        };
        StoredEntity {
            entity,
            etag: String::new(),
        }
    }

    fn build(entities: &[StoredEntity]) -> SearchIndex {
        let idx = SearchIndex::new().unwrap();
        idx.replace_all(entities).unwrap();
        idx
    }

    #[test]
    fn finds_match_in_title() {
        let entities = vec![ev(
            "shop",
            "order-placed",
            "Order placed",
            "Customer placed an order",
        )];
        let idx = build(&entities);
        let hits = idx.search("order", &[], &[], 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].0, entities[0].entity);
        assert!(hits[0].1 > 0.0);
    }

    #[test]
    fn finds_match_in_body() {
        let entities = vec![ev("shop", "x", "Other", "specific phrase here")];
        let idx = build(&entities);
        let hits = idx.search("phrase", &[], &[], 10).unwrap();
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn empty_query_returns_nothing() {
        let entities = vec![ev("shop", "x", "Order placed", "doc")];
        let idx = build(&entities);
        let hits = idx.search("", &[], &[], 10).unwrap();
        assert_eq!(
            hits,
            [] as [(trogon_atlas_proto::Entity, f32, std::string::String); 0]
        );
    }

    #[test]
    fn kind_filter_excludes_other_kinds() {
        let entities = vec![ev("shop", "x", "Order placed", "")];
        let idx = build(&entities);
        let hits = idx.search("order", &["command".into()], &[], 10).unwrap();
        assert_eq!(
            hits,
            [] as [(trogon_atlas_proto::Entity, f32, std::string::String); 0]
        );
        let hits = idx.search("order", &["event".into()], &[], 10).unwrap();
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn namespace_filter_excludes_other_namespaces() {
        let entities = vec![ev("shop", "x", "Order placed", "")];
        let idx = build(&entities);
        let hits = idx.search("order", &[], &["other".into()], 10).unwrap();
        assert_eq!(
            hits,
            [] as [(trogon_atlas_proto::Entity, f32, std::string::String); 0]
        );
        let hits = idx.search("order", &[], &["shop".into()], 10).unwrap();
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn higher_score_for_more_matches() {
        let entities = vec![
            ev("shop", "a", "Order placed", "order order order"),
            ev("shop", "b", "Other", "order"),
        ];
        let idx = build(&entities);
        let hits = idx.search("order", &[], &[], 10).unwrap();
        assert_eq!(hits.len(), 2);
        assert!(hits[0].1 >= hits[1].1);
    }

    #[test]
    fn apply_put_makes_new_entity_searchable() {
        let idx = build(&[]);
        assert_eq!(
            idx.search("order", &[], &[], 10).unwrap(),
            [] as [(trogon_atlas_proto::Entity, f32, std::string::String); 0]
        );
        let se = ev("shop", "order-placed", "Order placed", "");
        idx.apply_put(&se.entity).unwrap();
        assert_eq!(idx.search("order", &[], &[], 10).unwrap().len(), 1);
    }

    #[test]
    fn apply_put_replaces_existing_doc() {
        // Titles deliberately share no tokens with the slug so the old
        // title's disappearance is observable.
        let se = ev("shop", "order-placed", "Initial review", "");
        let idx = build(&[se]);
        let updated = ev("shop", "order-placed", "Final approval", "");
        idx.apply_put(&updated.entity).unwrap();
        assert_eq!(
            idx.search("initial", &[], &[], 10).unwrap(),
            [] as [(trogon_atlas_proto::Entity, f32, std::string::String); 0]
        );
        let hits = idx.search("approval", &[], &[], 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].0, updated.entity);
    }

    #[test]
    fn apply_delete_removes_entity() {
        let se = ev("shop", "order-placed", "Order placed", "");
        let idx = build(&[se]);
        idx.apply_delete(
            pb::EntityKind::Event,
            &pb::Id {
                namespace: "shop".into(),
                slug: "order-placed".into(),
                version: 0,
            },
        )
        .unwrap();
        assert_eq!(
            idx.search("order", &[], &[], 10).unwrap(),
            [] as [(trogon_atlas_proto::Entity, f32, std::string::String); 0]
        );
    }
}

#[cfg(test)]
mod view_version_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use std::sync::Arc;

    use trogon_atlas_proto as pb;

    use super::*;

    fn stored(namespace: &str, slug: &str, etag: &str) -> StoredEntity {
        StoredEntity {
            entity: pb::Entity {
                system: None,
                kind: Some(pb::entity::Kind::Event(pb::Event {
                    id: Some(pb::Id {
                        namespace: namespace.into(),
                        slug: slug.into(),
                        version: 1,
                    }),
                    title: slug.into(),
                    ..Default::default()
                })),
            },
            etag: etag.into(),
        }
    }

    fn index() -> Arc<SearchIndex> {
        Arc::new(SearchIndex::new().unwrap())
    }

    #[test]
    fn the_same_view_hashes_the_same() {
        let a = [stored("shop", "placed", "1"), stored("shop", "paid", "2")];
        let b = [stored("shop", "placed", "1"), stored("shop", "paid", "2")];
        assert_eq!(ViewVersion::of(&a), ViewVersion::of(&b));
    }

    /// The store's enumeration order is not part of a view's identity. If it
    /// were, the cache would miss at random.
    #[test]
    fn enumeration_order_does_not_change_the_version() {
        let forward = [stored("shop", "placed", "1"), stored("shop", "paid", "2")];
        let reversed = [stored("shop", "paid", "2"), stored("shop", "placed", "1")];
        assert_eq!(ViewVersion::of(&forward), ViewVersion::of(&reversed));
    }

    /// The whole point: one write to one entity must produce a new version,
    /// or a stale index gets served.
    #[test]
    fn one_changed_etag_changes_the_version() {
        let before = [stored("shop", "placed", "1")];
        let after = [stored("shop", "placed", "2")];
        assert_ne!(ViewVersion::of(&before), ViewVersion::of(&after));
    }

    #[test]
    fn adding_and_removing_an_entity_changes_the_version() {
        let one = [stored("shop", "placed", "1")];
        let two = [stored("shop", "placed", "1"), stored("shop", "paid", "1")];
        assert_ne!(ViewVersion::of(&one), ViewVersion::of(&two));
        assert_ne!(ViewVersion::of(&two), ViewVersion::of(&[]));
    }

    /// Length prefixes exist so a key's tail cannot masquerade as an etag's
    /// head. Without them these two views would hash alike.
    #[test]
    fn a_key_and_etag_boundary_cannot_be_shifted() {
        let a = [StoredEntity {
            etag: "b".into(),
            ..stored("shop", "placed", "")
        }];
        let b = [StoredEntity {
            etag: String::new(),
            ..stored("shop", "placedb", "")
        }];
        assert_ne!(ViewVersion::of(&a), ViewVersion::of(&b));
    }

    /// An entity the index would skip must not move the version either, or
    /// the cache key would describe documents the index does not hold.
    #[test]
    fn an_unindexable_entity_is_invisible_to_the_version() {
        let with_junk = [
            stored("shop", "placed", "1"),
            StoredEntity {
                entity: pb::Entity {
                    system: None,
                    kind: None,
                },
                etag: "9".into(),
            },
        ];
        let without = [stored("shop", "placed", "1")];
        assert_eq!(ViewVersion::of(&with_junk), ViewVersion::of(&without));
    }

    #[test]
    fn an_empty_view_has_a_version() {
        assert_eq!(ViewVersion::of(&[]), ViewVersion::of(&[]));
        assert_ne!(
            ViewVersion::of(&[]),
            ViewVersion::of(&[stored("a", "b", "1")])
        );
    }

    #[test]
    fn a_hit_requires_both_the_branch_and_the_version() {
        let cache = BranchIndexCache::default();
        let version = ViewVersion::of(&[stored("shop", "placed", "1")]);
        let other = ViewVersion::of(&[stored("shop", "placed", "2")]);
        cache.insert("feature".into(), version.clone(), index());

        assert!(cache.get("feature", &version).is_some());
        assert!(
            cache.get("feature", &other).is_none(),
            "a moved branch must not be served its previous index"
        );
        assert!(
            cache.get("other-branch", &version).is_none(),
            "two branches that happen to share a view are still two branches"
        );
    }

    /// A branch's view moves forward, so keeping its older index would be
    /// dead weight competing for a cache slot.
    #[test]
    fn re_inserting_a_branch_replaces_its_entry() {
        let cache = BranchIndexCache::default();
        let first = ViewVersion::of(&[stored("shop", "placed", "1")]);
        let second = ViewVersion::of(&[stored("shop", "placed", "2")]);
        cache.insert("feature".into(), first.clone(), index());
        cache.insert("feature".into(), second.clone(), index());

        assert_eq!(cache.len(), 1);
        assert!(cache.get("feature", &first).is_none());
        assert!(cache.get("feature", &second).is_some());
    }

    #[test]
    fn the_least_recently_used_branch_is_evicted() {
        let cache = BranchIndexCache::new(2);
        let v = ViewVersion::of(&[stored("shop", "placed", "1")]);
        cache.insert("a".into(), v.clone(), index());
        cache.insert("b".into(), v.clone(), index());
        // Touching `a` makes `b` the least recently used.
        assert!(cache.get("a", &v).is_some());
        cache.insert("c".into(), v.clone(), index());

        assert_eq!(cache.len(), 2);
        assert!(cache.get("a", &v).is_some());
        assert!(cache.get("c", &v).is_some());
        assert!(cache.get("b", &v).is_none());
    }

    #[test]
    fn a_zero_capacity_cache_still_holds_one_entry() {
        let cache = BranchIndexCache::new(0);
        let v = ViewVersion::of(&[stored("shop", "placed", "1")]);
        cache.insert("a".into(), v.clone(), index());
        assert!(cache.get("a", &v).is_some());
    }
}
