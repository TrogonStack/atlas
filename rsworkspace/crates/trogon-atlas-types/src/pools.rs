use std::{
    collections::{HashMap, HashSet},
    fmt,
    hash::Hash,
    sync::{Arc, Mutex, PoisonError},
};

use prost_reflect::DescriptorPool;
use protox::{file::GoogleFileResolver, Compiler};

use crate::{
    bundle::{CompileLimits, TypeLibraryDigest},
    compile::{CompileError, TypeLibrarySource},
    dependencies::{resolve_dependencies, DependencyError, LibraryNode},
    package::ProtoPackagePrefix,
    runner::CompileRunner,
};

const WELL_KNOWN_FILES: &[&str] = &[
    "google/protobuf/any.proto",
    "google/protobuf/api.proto",
    "google/protobuf/descriptor.proto",
    "google/protobuf/duration.proto",
    "google/protobuf/empty.proto",
    "google/protobuf/field_mask.proto",
    "google/protobuf/source_context.proto",
    "google/protobuf/struct.proto",
    "google/protobuf/timestamp.proto",
    "google/protobuf/type.proto",
    "google/protobuf/wrappers.proto",
];

const DEFAULT_CAPACITY: usize = 64;

/// Where a pool's libraries live: one namespace on one branch, or on the
/// trunk when `branch` is absent.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PoolScope {
    namespace: String,
    branch: Option<String>,
}

impl PoolScope {
    pub fn new(namespace: impl Into<String>, branch: Option<impl Into<String>>) -> Self {
        Self {
            namespace: namespace.into(),
            branch: branch.map(Into::into),
        }
    }
}

/// A live library as a pool sees it.
#[derive(Debug, Clone)]
pub struct PoolMember<K> {
    pub key: K,
    pub source: TypeLibrarySource,
    pub dependencies: Vec<K>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct PoolKey<K> {
    scope: PoolScope,
    libraries: Vec<(K, TypeLibraryDigest)>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PoolError<K: fmt::Debug + fmt::Display> {
    #[error("type libraries {first} ({first_prefix}) and {second} ({second_prefix}) own overlapping packages")]
    PrefixOverlap {
        first: K,
        first_prefix: ProtoPackagePrefix,
        second: K,
        second_prefix: ProtoPackagePrefix,
    },
    #[error("type library {library}: {error}")]
    Dependency {
        library: K,
        error: DependencyError<K>,
    },
    #[error("type library {library}: {error}")]
    Compile { library: K, error: CompileError },
    #[error("type library {library} cannot join the pool: {message}")]
    Descriptor { library: K, message: String },
}

/// Descriptor pools of every live tenant library in a scope, stacked on the
/// built-in atlas and well-known types. Pools are rebuilt only when the set
/// of live library digests changes.
#[derive(Debug)]
pub struct TypePools<K> {
    builtins: DescriptorPool,
    runner: CompileRunner,
    limits: CompileLimits,
    capacity: usize,
    cache: Mutex<Cache<K>>,
}

#[derive(Debug)]
struct Cache<K> {
    tick: u64,
    entries: HashMap<PoolKey<K>, (Arc<DescriptorPool>, u64)>,
}

impl<K> TypePools<K>
where
    K: Clone + Eq + Hash + Ord + fmt::Debug + fmt::Display,
{
    pub fn new(runner: CompileRunner, limits: CompileLimits) -> Self {
        Self::with_capacity(runner, limits, DEFAULT_CAPACITY)
    }

    pub fn with_capacity(runner: CompileRunner, limits: CompileLimits, capacity: usize) -> Self {
        Self {
            builtins: builtin_pool(),
            runner,
            limits,
            capacity: capacity.max(1),
            cache: Mutex::new(Cache {
                tick: 0,
                entries: HashMap::new(),
            }),
        }
    }

    pub fn builtins(&self) -> &DescriptorPool {
        &self.builtins
    }

    pub async fn get_or_build(
        &self,
        scope: PoolScope,
        members: Vec<PoolMember<K>>,
    ) -> Result<Arc<DescriptorPool>, PoolError<K>> {
        let mut libraries: Vec<(K, TypeLibraryDigest)> = members
            .iter()
            .map(|m| (m.key.clone(), m.source.digest()))
            .collect();
        libraries.sort();
        let key = PoolKey { scope, libraries };
        if let Some(pool) = self.cached(&key) {
            return Ok(pool);
        }
        let pool = Arc::new(self.build(members).await?);
        self.insert(key, Arc::clone(&pool));
        Ok(pool)
    }

    fn cached(&self, key: &PoolKey<K>) -> Option<Arc<DescriptorPool>> {
        let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        cache.tick += 1;
        let tick = cache.tick;
        cache.entries.get_mut(key).map(|(pool, used)| {
            *used = tick;
            Arc::clone(pool)
        })
    }

    fn insert(&self, key: PoolKey<K>, pool: Arc<DescriptorPool>) {
        let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        cache.tick += 1;
        let tick = cache.tick;
        cache.entries.insert(key, (pool, tick));
        while cache.entries.len() > self.capacity {
            let Some(oldest) = cache
                .entries
                .iter()
                .min_by_key(|(_, (_, used))| *used)
                .map(|(k, _)| k.clone())
            else {
                break;
            };
            cache.entries.remove(&oldest);
        }
    }

    async fn build(&self, mut members: Vec<PoolMember<K>>) -> Result<DescriptorPool, PoolError<K>> {
        members.sort_by(|a, b| a.key.cmp(&b.key));
        reject_overlaps(&members)?;

        let by_key: HashMap<&K, &PoolMember<K>> = members.iter().map(|m| (&m.key, m)).collect();
        let lookup = |k: &K| {
            by_key.get(k).map(|m| LibraryNode {
                source: m.source.clone(),
                dependencies: m.dependencies.clone(),
            })
        };

        let mut pool = self.builtins.clone();
        let mut added: HashSet<K> = HashSet::new();
        for member in &members {
            let closure =
                resolve_dependencies(&member.key, &member.dependencies, lookup, &self.limits)
                    .map_err(|error| PoolError::Dependency {
                        library: member.key.clone(),
                        error,
                    })?;
            let mut chain: Vec<(K, TypeLibrarySource, Vec<TypeLibrarySource>)> = Vec::new();
            for (index, (key, source)) in closure.iter().enumerate() {
                let deps = closure[..index]
                    .iter()
                    .map(|(_, s)| s.clone())
                    .collect::<Vec<_>>();
                chain.push((key.clone(), source.clone(), deps));
            }
            chain.push((
                member.key.clone(),
                member.source.clone(),
                closure.into_iter().map(|(_, s)| s).collect(),
            ));
            for (key, source, deps) in chain {
                if added.contains(&key) {
                    continue;
                }
                let compiled = self.runner.compile(source, deps).await.map_err(|error| {
                    PoolError::Compile {
                        library: key.clone(),
                        error,
                    }
                })?;
                pool.add_file_descriptor_set(compiled.file_descriptor_set().clone())
                    .map_err(|e| PoolError::Descriptor {
                        library: key.clone(),
                        message: e.to_string(),
                    })?;
                added.insert(key);
            }
        }
        Ok(pool)
    }
}

fn reject_overlaps<K>(members: &[PoolMember<K>]) -> Result<(), PoolError<K>>
where
    K: Clone + fmt::Debug + fmt::Display,
{
    for (index, first) in members.iter().enumerate() {
        for second in &members[index + 1..] {
            if first.source.prefix().overlaps(second.source.prefix()) {
                return Err(PoolError::PrefixOverlap {
                    first: first.key.clone(),
                    first_prefix: first.source.prefix().clone(),
                    second: second.key.clone(),
                    second_prefix: second.source.prefix().clone(),
                });
            }
        }
    }
    Ok(())
}

#[allow(clippy::expect_used)]
fn builtin_pool() -> DescriptorPool {
    let mut compiler = Compiler::with_file_resolver(GoogleFileResolver::new());
    compiler.include_imports(true);
    compiler
        .open_files(WELL_KNOWN_FILES)
        .expect("protox bundles every well-known type");
    let mut pool = compiler.descriptor_pool();
    pool.decode_file_descriptor_set(trogon_atlas_proto::FILE_DESCRIPTOR_SET)
        .expect("the generated atlas descriptor set is valid");
    pool
}
