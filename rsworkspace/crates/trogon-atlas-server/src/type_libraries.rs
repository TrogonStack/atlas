use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fmt,
    sync::Arc,
};

use prost::Message as _;
use prost_reflect::{DescriptorPool, DynamicMessage, FileDescriptor, ReflectMessage as _, Value};
use tonic::{Code, Status};
use tonic_types::{ErrorDetails, StatusExt};
use trogon_atlas_proto as pb;
use trogon_atlas_store::{store::ListFilter, Store};
use trogon_atlas_types::{
    resolve_dependencies, BreakingChange, CompileError, CompileLimits, CompileRunner,
    CompiledLibrary, DependencyError, Diagnostic, LibraryNode, PoolError, PoolMember, PoolScope,
    ProtoPackagePrefix, SourceBundle, TypeLibrarySource, TypePools, TypeUrl,
};

use crate::conv::{reason, store_err, ERROR_DOMAIN};

/// One version of one type library inside a namespace.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LibraryVersion {
    slug: String,
    version: u64,
}

impl LibraryVersion {
    fn of(id: &pb::Id) -> Self {
        Self {
            slug: id.slug.clone(),
            version: id.version,
        }
    }
}

impl fmt::Display for LibraryVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@v{}", self.slug, self.version)
    }
}

/// The type libraries of one namespace as a write saw them. A write that was
/// admitted against one view must not land on a different one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeLibraryAdmission {
    namespace: String,
    seen: BTreeMap<LibraryVersion, String>,
}

/// Type library puts and deletes that are meant to land together.
#[derive(Debug, Clone, Default)]
pub struct TypeLibraryChanges {
    by_namespace: BTreeMap<String, Vec<Change>>,
}

#[derive(Debug, Clone)]
enum Change {
    Put(Box<pb::TypeLibrary>),
    Delete(LibraryVersion),
}

impl TypeLibraryChanges {
    /// Records a put. Entities of any other kind are ignored.
    pub fn put(&mut self, entity: &pb::Entity) {
        if let Some(pb::entity::Kind::TypeLibrary(library)) = &entity.kind {
            if let Some(id) = &library.id {
                self.by_namespace
                    .entry(id.namespace.clone())
                    .or_default()
                    .push(Change::Put(Box::new(library.clone())));
            }
        }
    }

    /// Records a delete. Entities of any other kind are ignored.
    pub fn delete(&mut self, kind: pb::EntityKind, id: &pb::Id) {
        if kind == pb::EntityKind::TypeLibrary {
            self.by_namespace
                .entry(id.namespace.clone())
                .or_default()
                .push(Change::Delete(LibraryVersion::of(id)));
        }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_namespace.is_empty()
    }
}

/// Compiles and checks tenant type libraries before the store accepts them,
/// and serves the types the live ones declare.
#[derive(Debug, Clone)]
pub struct TypeLibraryGate {
    runner: CompileRunner,
    limits: CompileLimits,
    pools: Arc<TypePools<LibraryVersion>>,
}

impl Default for TypeLibraryGate {
    fn default() -> Self {
        let runner = CompileRunner::default();
        let limits = CompileLimits::default();
        Self {
            pools: Arc::new(TypePools::new(runner.clone(), limits)),
            runner,
            limits,
        }
    }
}

struct Assessment {
    library: LibraryVersion,
    diagnostics: Vec<Diagnostic>,
    breaking_changes: Vec<BreakingChange>,
}

/// The live type libraries of one namespace, compiled together.
struct LivePool {
    pool: Arc<DescriptorPool>,
    owners: Vec<(ProtoPackagePrefix, pb::Id)>,
    admission: TypeLibraryAdmission,
}

#[derive(Clone)]
struct Registered {
    library: pb::TypeLibrary,
    etag: String,
}

impl TypeLibraryGate {
    /// Checks a write of `entity`. Returns `None` for any other kind.
    ///
    /// # Errors
    ///
    /// `InvalidArgument` when the library does not compile or names an
    /// unusable dependency, `AlreadyExists` when another live library owns an
    /// overlapping package prefix, and `FailedPrecondition` when a dependency
    /// does not exist or the change would break existing payloads.
    pub async fn admit(
        &self,
        store: &dyn Store,
        entity: &pb::Entity,
        branch: Option<&str>,
    ) -> Result<Option<TypeLibraryAdmission>, Status> {
        let Some(pb::entity::Kind::TypeLibrary(library)) = &entity.kind else {
            return Ok(None);
        };
        let id = library
            .id
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("type_library.id is required"))?;
        let registered = load(store, &id.namespace, branch).await?;
        let admission = TypeLibraryAdmission {
            namespace: id.namespace.clone(),
            seen: registered
                .iter()
                .map(|(k, r)| (k.clone(), r.etag.clone()))
                .collect(),
        };
        self.check_put(library, &registered, &registered).await?;
        Ok(Some(admission))
    }

    /// Checks type library writes that land together against the libraries
    /// already in `branch`: every put as [`Self::admit`] would, against the
    /// world after all of them land, and every delete for libraries that
    /// would be left depending on it.
    ///
    /// # Errors
    ///
    /// The statuses [`Self::admit`] returns, plus `FailedPrecondition` when a
    /// deleted library is still a dependency of another one.
    pub async fn verify(
        &self,
        store: &dyn Store,
        changes: &TypeLibraryChanges,
        branch: Option<&str>,
    ) -> Result<(), Status> {
        for (namespace, namespace_changes) in &changes.by_namespace {
            let before = load(store, namespace, branch).await?;
            let mut after = before.clone();
            let mut deleted = Vec::new();
            for change in namespace_changes {
                match change {
                    Change::Put(library) => {
                        if let Some(id) = &library.id {
                            after.insert(
                                LibraryVersion::of(id),
                                Registered {
                                    library: (**library).clone(),
                                    etag: String::new(),
                                },
                            );
                        }
                    }
                    Change::Delete(key) => {
                        after.remove(key);
                        deleted.push(key);
                    }
                }
            }
            reject_dependents(&deleted, &after)?;
            for change in namespace_changes {
                if let Change::Put(library) = change {
                    self.check_put(library, &before, &after).await?;
                }
            }
        }
        Ok(())
    }

    /// Re-reads the namespace under the mutation lock and refuses the write
    /// when any type library changed since it was admitted.
    ///
    /// # Errors
    ///
    /// `Aborted` when the namespace's type libraries moved.
    pub async fn confirm(
        &self,
        store: &dyn Store,
        admission: &TypeLibraryAdmission,
        branch: Option<&str>,
    ) -> Result<(), Status> {
        let now: BTreeMap<LibraryVersion, String> = load(store, &admission.namespace, branch)
            .await?
            .into_iter()
            .map(|(k, r)| (k, r.etag))
            .collect();
        if now == admission.seen {
            Ok(())
        } else {
            Err(Status::with_error_details(
                Code::Aborted,
                format!(
                    "type libraries in namespace {} changed while this write was being checked; retry",
                    admission.namespace
                ),
                ErrorDetails::with_error_info(
                    reason::STALE_PRECONDITION,
                    ERROR_DOMAIN,
                    HashMap::new(),
                ),
            ))
        }
    }

    async fn check_put(
        &self,
        library: &pb::TypeLibrary,
        before: &BTreeMap<LibraryVersion, Registered>,
        after: &BTreeMap<LibraryVersion, Registered>,
    ) -> Result<(), Status> {
        let assessment = self.assess(library, before, after).await?;
        if !assessment.diagnostics.is_empty() {
            return Err(diagnostics_err(&assessment.diagnostics));
        }
        reject_breaking(&assessment.library, &assessment.breaking_changes)
    }

    async fn assess(
        &self,
        library: &pb::TypeLibrary,
        before: &BTreeMap<LibraryVersion, Registered>,
        after: &BTreeMap<LibraryVersion, Registered>,
    ) -> Result<Assessment, Status> {
        let id = library
            .id
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("type_library.id is required"))?;
        let own = LibraryVersion::of(id);
        let source = self.source_of(library)?;
        let dependencies = dependency_keys(id, library)?;
        let mut assessment = Assessment {
            library: own.clone(),
            diagnostics: Vec::new(),
            breaking_changes: Vec::new(),
        };

        if let Some(existing) = before.get(&own) {
            let unchanged = self
                .source_of(&existing.library)
                .is_ok_and(|s| s.digest() == source.digest())
                && existing.library.dependencies == library.dependencies;
            if unchanged {
                return Ok(assessment);
            }
        }

        reject_overlap(&own, source.prefix(), after)?;

        let closure = self.closure(&own, &dependencies, after)?;
        let compiled = match self.runner.compile(source, closure).await {
            Ok(compiled) => compiled,
            Err(CompileError::Diagnostics(diagnostics)) => {
                assessment.diagnostics = diagnostics;
                return Ok(assessment);
            }
            Err(other) => return Err(compile_err(other)),
        };

        if let Some((previous_key, previous)) = previous_version(&own, before) {
            match self
                .compile_registered(&previous_key, previous, before)
                .await
            {
                Ok(previous) => {
                    assessment.breaking_changes = compiled.breaking_changes_since(&previous);
                }
                Err(error) => tracing::warn!(
                    library = %previous_key,
                    error = %error.message(),
                    "previous type library version no longer compiles; skipping compatibility check"
                ),
            }
        }
        Ok(assessment)
    }

    /// Checks `library` the way a write would, reporting what is wrong with
    /// its source as data instead of failing.
    ///
    /// # Errors
    ///
    /// The statuses [`Self::admit`] returns for anything other than compiler
    /// diagnostics and breaking changes.
    pub async fn dry_run(
        &self,
        store: &dyn Store,
        library: &pb::TypeLibrary,
        branch: Option<&str>,
    ) -> Result<pb::CompileTypeLibraryResponse, Status> {
        let id = library
            .id
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("library.id is required"))?;
        let registered = load(store, &id.namespace, branch).await?;
        let assessment = self.assess(library, &registered, &registered).await?;
        Ok(pb::CompileTypeLibraryResponse {
            diagnostics: assessment
                .diagnostics
                .into_iter()
                .map(|d| pb::TypeLibraryDiagnostic {
                    path: d.path,
                    line: d.line,
                    column: d.column,
                    message: d.message,
                })
                .collect(),
            compatibility_violations: assessment
                .breaking_changes
                .iter()
                .map(|c| pb::TypeLibraryCompatibilityViolation {
                    message: c.to_string(),
                })
                .collect(),
        })
    }

    /// Finds the message `type_url` names among the namespace's live type
    /// libraries and the built-in types.
    ///
    /// # Errors
    ///
    /// `NotFound` when no live library declares the message, and
    /// `FailedPrecondition` when the live libraries do not compile together.
    pub async fn resolve(
        &self,
        store: &dyn Store,
        namespace: &str,
        type_url: &TypeUrl,
        branch: Option<&str>,
    ) -> Result<pb::ResolveTypeResponse, Status> {
        let live = self.live_pool(store, namespace, &[], branch).await?;
        let message = live
            .pool
            .get_message_by_name(type_url.full_name())
            .ok_or_else(|| unknown_type(namespace, type_url))?;
        let library = if self.is_builtin(&message.parent_file()) {
            None
        } else {
            live.owner_of(message.package_name())
        };
        Ok(pb::ResolveTypeResponse {
            library,
            file_descriptor_set: file_closure([message.parent_file()]).encode_to_vec(),
        })
    }

    /// Every file and message the namespace's live type libraries declare.
    ///
    /// # Errors
    ///
    /// `FailedPrecondition` when the live libraries do not compile together.
    pub async fn descriptor_set(
        &self,
        store: &dyn Store,
        namespace: &str,
        branch: Option<&str>,
    ) -> Result<pb::GetTypeLibraryDescriptorSetResponse, Status> {
        let live = self.live_pool(store, namespace, &[], branch).await?;
        let files: Vec<FileDescriptor> = live
            .pool
            .files()
            .filter(|file| !self.is_builtin(file))
            .collect();
        let messages = live
            .pool
            .all_messages()
            .filter(|message| !self.is_builtin(&message.parent_file()))
            .map(|message| pb::TypeLibraryMessage {
                full_name: message.full_name().to_owned(),
                library: live.owner_of(message.package_name()),
            })
            .collect();
        Ok(pb::GetTypeLibraryDescriptorSetResponse {
            file_descriptor_set: file_closure(files).encode_to_vec(),
            messages,
        })
    }

    /// Checks that the payload type an Event, Command or ReadModel names is
    /// one its namespace can resolve, and that any payload bytes it carries
    /// decode as that type. `pending` are type libraries written alongside
    /// the entity. The built-in `Schema` type is left to the existing checks.
    /// The returned admission must be confirmed under the mutation lock.
    ///
    /// # Errors
    ///
    /// `InvalidArgument` when the type is unknown or the bytes do not decode
    /// as it, and `FailedPrecondition` when the live libraries do not compile
    /// together.
    pub async fn check_schema(
        &self,
        store: &dyn Store,
        entity: &pb::Entity,
        pending: &[&pb::TypeLibrary],
        branch: Option<&str>,
    ) -> Result<Option<TypeLibraryAdmission>, Status> {
        let Some(schema) = trogon_atlas_core::schema::entity_schema(entity) else {
            return Ok(None);
        };
        if trogon_atlas_core::schema::is_schema(schema) {
            return Ok(None);
        }
        let Some(id) = trogon_atlas_core::refs::entity_id(entity) else {
            return Ok(None);
        };
        let type_url = TypeUrl::parse(&schema.type_url)
            .map_err(|e| invalid(format!("schema.type_url: {e}")))?;
        let live = self
            .live_pool(store, &id.namespace, pending, branch)
            .await?;
        let message = live
            .pool
            .get_message_by_name(type_url.full_name())
            .ok_or_else(|| {
                invalid(format!(
                    "schema.type_url names {}, which no live type library in namespace {} declares",
                    type_url.full_name(),
                    id.namespace
                ))
            })?;
        if schema.value.is_empty() {
            return Ok(Some(live.admission));
        }
        let decoded = DynamicMessage::decode(message, schema.value.as_slice()).map_err(|e| {
            invalid(format!(
                "schema.value does not decode as {}: {e}",
                type_url.full_name()
            ))
        })?;
        if let Some(path) = first_unknown_field(&decoded) {
            return Err(invalid(format!(
                "schema.value carries field {path}, which {} does not declare",
                type_url.full_name()
            )));
        }
        Ok(Some(live.admission))
    }

    fn is_builtin(&self, file: &FileDescriptor) -> bool {
        self.pools
            .builtins()
            .get_file_by_name(file.name())
            .is_some()
    }

    async fn live_pool(
        &self,
        store: &dyn Store,
        namespace: &str,
        pending: &[&pb::TypeLibrary],
        branch: Option<&str>,
    ) -> Result<LivePool, Status> {
        let mut registered = load(store, namespace, branch).await?;
        let admission = TypeLibraryAdmission {
            namespace: namespace.to_owned(),
            seen: registered
                .iter()
                .map(|(k, r)| (k.clone(), r.etag.clone()))
                .collect(),
        };
        for library in pending {
            if let Some(id) = library.id.as_ref().filter(|id| id.namespace == namespace) {
                registered.insert(
                    LibraryVersion::of(id),
                    Registered {
                        library: (*library).clone(),
                        etag: String::new(),
                    },
                );
            }
        }
        let live = latest_per_slug(&registered);
        let mut members = Vec::with_capacity(live.len());
        let mut owners = Vec::with_capacity(live.len());
        for (key, entry) in live.values() {
            let Some(id) = entry.library.id.as_ref() else {
                continue;
            };
            let source = self.source_of(&entry.library)?;
            owners.push((source.prefix().clone(), id.clone()));
            let dependencies = dependency_keys(id, &entry.library)?
                .into_iter()
                .map(|dependency| {
                    live.get(dependency.slug.as_str())
                        .map_or(dependency, |(current, _)| (*current).clone())
                })
                .collect();
            members.push(PoolMember {
                key: (*key).clone(),
                source,
                dependencies,
            });
        }
        let pool = self
            .pools
            .get_or_build(PoolScope::new(namespace, branch), members)
            .await
            .map_err(|e| pool_err(namespace, e))?;
        Ok(LivePool {
            pool,
            owners,
            admission,
        })
    }

    fn source_of(&self, library: &pb::TypeLibrary) -> Result<TypeLibrarySource, Status> {
        let slug = library.id.as_ref().map_or("", |id| id.slug.as_str());
        let prefix = ProtoPackagePrefix::parse(slug).map_err(|e| {
            invalid(format!(
                "type library slug must be the protobuf package prefix it owns: {e}"
            ))
        })?;
        let bundle = SourceBundle::from_proto(&library.files, &self.limits)
            .map_err(|e| invalid(format!("type library files: {e}")))?;
        Ok(TypeLibrarySource::new(prefix, bundle))
    }

    fn closure(
        &self,
        own: &LibraryVersion,
        dependencies: &[LibraryVersion],
        registered: &BTreeMap<LibraryVersion, Registered>,
    ) -> Result<Vec<TypeLibrarySource>, Status> {
        let lookup = |key: &LibraryVersion| {
            let entry = registered.get(key)?;
            let id = entry.library.id.as_ref()?;
            Some(LibraryNode {
                source: self.source_of(&entry.library).ok()?,
                dependencies: dependency_keys(id, &entry.library).ok()?,
            })
        };
        resolve_dependencies(own, dependencies, lookup, &self.limits)
            .map(|order| order.into_iter().map(|(_, source)| source).collect())
            .map_err(|e| match e {
                DependencyError::Missing(_) => Status::with_error_details(
                    Code::FailedPrecondition,
                    format!("type library {own}: {e}"),
                    ErrorDetails::with_error_info(reason::NOT_FOUND, ERROR_DOMAIN, HashMap::new()),
                ),
                other => invalid(format!("type library {own}: {other}")),
            })
    }

    async fn compile(
        &self,
        source: TypeLibrarySource,
        dependencies: Vec<TypeLibrarySource>,
    ) -> Result<CompiledLibrary, Status> {
        self.runner
            .compile(source, dependencies)
            .await
            .map_err(compile_err)
    }

    async fn compile_registered(
        &self,
        key: &LibraryVersion,
        entry: &Registered,
        registered: &BTreeMap<LibraryVersion, Registered>,
    ) -> Result<CompiledLibrary, Status> {
        let id = entry
            .library
            .id
            .as_ref()
            .ok_or_else(|| Status::internal("stored type library has no id"))?;
        let source = self.source_of(&entry.library)?;
        let dependencies = dependency_keys(id, &entry.library)?;
        let closure = self.closure(key, &dependencies, registered)?;
        self.compile(source, closure).await
    }
}

impl LivePool {
    fn owner_of(&self, package: &str) -> Option<pb::Id> {
        self.owners
            .iter()
            .find(|(prefix, _)| prefix.owns(package))
            .map(|(_, id)| id.clone())
    }
}

fn file_closure(roots: impl IntoIterator<Item = FileDescriptor>) -> prost_types::FileDescriptorSet {
    fn visit(
        file: &FileDescriptor,
        seen: &mut HashSet<String>,
        out: &mut Vec<prost_types::FileDescriptorProto>,
    ) {
        if !seen.insert(file.name().to_owned()) {
            return;
        }
        for dependency in file.dependencies() {
            visit(&dependency, seen, out);
        }
        out.push(file.file_descriptor_proto().clone());
    }
    let mut seen = HashSet::new();
    let mut file = Vec::new();
    for root in roots {
        visit(&root, &mut seen, &mut file);
    }
    prost_types::FileDescriptorSet { file }
}

fn first_unknown_field(message: &DynamicMessage) -> Option<String> {
    let name = message.descriptor().full_name().to_owned();
    if let Some(field) = message.unknown_fields().next() {
        return Some(format!("{name}.{}", field.number()));
    }
    message
        .fields()
        .find_map(|(_, value)| unknown_in_value(value))
}

fn unknown_in_value(value: &Value) -> Option<String> {
    match value {
        Value::Message(message) => first_unknown_field(message),
        Value::List(values) => values.iter().find_map(unknown_in_value),
        Value::Map(entries) => entries.values().find_map(unknown_in_value),
        _ => None,
    }
}

fn unknown_type(namespace: &str, type_url: &TypeUrl) -> Status {
    Status::with_error_details(
        Code::NotFound,
        format!(
            "no live type library in namespace {namespace} declares {}",
            type_url.full_name()
        ),
        ErrorDetails::with_error_info(reason::NOT_FOUND, ERROR_DOMAIN, HashMap::new()),
    )
}

fn pool_err(namespace: &str, error: PoolError<LibraryVersion>) -> Status {
    match error {
        PoolError::Compile {
            error: error @ (CompileError::TimedOut(_) | CompileError::Unavailable),
            ..
        } => compile_err(error),
        other => Status::failed_precondition(format!(
            "the live type libraries in namespace {namespace} do not compile together: {other}"
        )),
    }
}

async fn load(
    store: &dyn Store,
    namespace: &str,
    branch: Option<&str>,
) -> Result<BTreeMap<LibraryVersion, Registered>, Status> {
    let namespaces = [namespace.to_owned()];
    let stored = store
        .list(
            ListFilter {
                kinds: &[pb::EntityKind::TypeLibrary],
                namespaces: &namespaces,
                latest_versions_only: false,
            },
            None,
            branch,
        )
        .await
        .map_err(store_err)?;
    Ok(stored
        .into_iter()
        .filter_map(|s| match s.entity.kind {
            Some(pb::entity::Kind::TypeLibrary(library)) => {
                let key = LibraryVersion::of(library.id.as_ref()?);
                Some((
                    key,
                    Registered {
                        library,
                        etag: s.etag,
                    },
                ))
            }
            _ => None,
        })
        .collect())
}

fn dependency_keys(id: &pb::Id, library: &pb::TypeLibrary) -> Result<Vec<LibraryVersion>, Status> {
    library
        .dependencies
        .iter()
        .enumerate()
        .map(|(i, dep)| {
            let dep_id = dep.id.as_ref().ok_or_else(|| {
                invalid(format!("type_library.dependencies[{i}].id is required"))
            })?;
            if dep_id.namespace != id.namespace {
                return Err(invalid(format!(
                    "type_library.dependencies[{i}] is in namespace {:?}; type libraries may only depend on libraries in their own namespace {:?}",
                    dep_id.namespace, id.namespace
                )));
            }
            if dep_id.slug == id.slug {
                return Err(invalid(format!(
                    "type_library.dependencies[{i}] names another version of this same library"
                )));
            }
            Ok(LibraryVersion::of(dep_id))
        })
        .collect()
}

fn latest_per_slug(
    registered: &BTreeMap<LibraryVersion, Registered>,
) -> BTreeMap<&str, (&LibraryVersion, &Registered)> {
    let mut latest: BTreeMap<&str, (&LibraryVersion, &Registered)> = BTreeMap::new();
    for (key, entry) in registered {
        latest.insert(key.slug.as_str(), (key, entry));
    }
    latest
}

fn reject_overlap(
    own: &LibraryVersion,
    prefix: &ProtoPackagePrefix,
    registered: &BTreeMap<LibraryVersion, Registered>,
) -> Result<(), Status> {
    for (slug, (key, _)) in latest_per_slug(registered) {
        if slug == own.slug {
            continue;
        }
        let Ok(other) = ProtoPackagePrefix::parse(slug) else {
            continue;
        };
        if prefix.overlaps(&other) {
            return Err(Status::with_error_details(
                Code::AlreadyExists,
                format!(
                    "type library {own} owns packages under {prefix}, which overlap the packages under {other} owned by {key}"
                ),
                ErrorDetails::with_error_info(
                    reason::ALREADY_EXISTS,
                    ERROR_DOMAIN,
                    HashMap::new(),
                ),
            ));
        }
    }
    Ok(())
}

/// Refuses to remove type libraries that entity schemas still name.
///
/// # Errors
///
/// `FailedPrecondition` listing each referrer when `users` is not empty.
pub fn reject_schema_users(users: &[pb::Reference]) -> Result<(), Status> {
    if users.is_empty() {
        return Ok(());
    }
    let mut details = ErrorDetails::new();
    let mut reasons = Vec::new();
    for user in users {
        let (Some(from), Some(to)) = (
            user.from.as_ref().and_then(|r| r.id.as_ref()),
            user.to.as_ref().and_then(|r| r.id.as_ref()),
        ) else {
            continue;
        };
        let referrer = format!("{}/{}@v{}", from.namespace, from.slug, from.version);
        let library = LibraryVersion::of(to);
        details.add_precondition_failure_violation(
            "TYPE_LIBRARY_SCHEMA",
            referrer.clone(),
            format!("schema names a type from {library}"),
        );
        reasons.push(format!("{referrer} uses a type from {library}"));
    }
    Err(Status::with_error_details(
        Code::FailedPrecondition,
        format!(
            "entity schemas still use a type library being removed; repoint them first: {}",
            reasons.join("; ")
        ),
        details,
    ))
}

fn reject_dependents(
    deleted: &[&LibraryVersion],
    after: &BTreeMap<LibraryVersion, Registered>,
) -> Result<(), Status> {
    let mut details = ErrorDetails::new();
    let mut reasons = Vec::new();
    for (key, entry) in after {
        let Some(id) = entry.library.id.as_ref() else {
            continue;
        };
        for dependency in dependency_keys(id, &entry.library).unwrap_or_default() {
            if deleted.contains(&&dependency) {
                details.add_precondition_failure_violation(
                    "TYPE_LIBRARY_DEPENDENCY",
                    key.to_string(),
                    format!("depends on {dependency}"),
                );
                reasons.push(format!("{key} depends on {dependency}"));
            }
        }
    }
    if reasons.is_empty() {
        return Ok(());
    }
    Err(Status::with_error_details(
        Code::FailedPrecondition,
        format!(
            "type libraries still depend on a library being removed; repoint or remove them first: {}",
            reasons.join("; ")
        ),
        details,
    ))
}

fn previous_version<'a>(
    own: &LibraryVersion,
    registered: &'a BTreeMap<LibraryVersion, Registered>,
) -> Option<(LibraryVersion, &'a Registered)> {
    if let Some(entry) = registered.get(own) {
        return Some((own.clone(), entry));
    }
    registered
        .iter()
        .filter(|(k, _)| k.slug == own.slug && k.version < own.version)
        .max_by_key(|(k, _)| k.version)
        .map(|(k, entry)| (k.clone(), entry))
}

fn reject_breaking(own: &LibraryVersion, changes: &[BreakingChange]) -> Result<(), Status> {
    if changes.is_empty() {
        return Ok(());
    }
    let mut details =
        ErrorDetails::with_error_info(reason::BREAKING_CHANGE, ERROR_DOMAIN, HashMap::new());
    for change in changes {
        details.add_precondition_failure_violation(
            "TYPE_LIBRARY_COMPATIBILITY",
            own.to_string(),
            change.to_string(),
        );
    }
    let reasons = changes
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ");
    Err(Status::with_error_details(
        Code::FailedPrecondition,
        format!(
            "type library {own} would break existing payloads; publish the change under a new package and supersede this library instead: {reasons}"
        ),
        details,
    ))
}

fn compile_err(error: CompileError) -> Status {
    match error {
        CompileError::Diagnostics(diagnostics) => diagnostics_err(&diagnostics),
        CompileError::TimedOut(_) => Status::with_error_details(
            Code::DeadlineExceeded,
            error.to_string(),
            ErrorDetails::with_error_info(reason::UNAVAILABLE, ERROR_DOMAIN, HashMap::new()),
        ),
        CompileError::Unavailable => Status::with_error_details(
            Code::Unavailable,
            error.to_string(),
            ErrorDetails::with_error_info(reason::UNAVAILABLE, ERROR_DOMAIN, HashMap::new()),
        ),
    }
}

fn diagnostics_err(diagnostics: &[Diagnostic]) -> Status {
    let mut details =
        ErrorDetails::with_error_info(reason::VALIDATION_FAILED, ERROR_DOMAIN, HashMap::new());
    for d in diagnostics {
        details.add_bad_request_violation(
            format!("{}:{}:{}", d.path, d.line, d.column),
            d.message.clone(),
        );
    }
    Status::with_error_details(
        Code::InvalidArgument,
        CompileError::Diagnostics(diagnostics.to_vec()).to_string(),
        details,
    )
}

fn invalid(message: String) -> Status {
    Status::with_error_details(
        Code::InvalidArgument,
        message,
        ErrorDetails::with_error_info(reason::VALIDATION_FAILED, ERROR_DOMAIN, HashMap::new()),
    )
}
