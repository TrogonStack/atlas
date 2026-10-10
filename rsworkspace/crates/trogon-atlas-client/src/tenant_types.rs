//! Tenant types for JSON transcoding: an `Any` naming a tenant message only
//! converts to and from JSON once the type libraries declaring it are
//! layered onto the built-in descriptors. They come from the server for the
//! namespaces a payload touches, and from source for libraries written
//! alongside it, so one batch can carry a library and its first users.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context as _, Result};
use prost::Message as _;
use prost_types::{FileDescriptorProto, FileDescriptorSet};
use serde_json::Value;
use trogon_atlas_core::transcode::TranscodePool;
use trogon_atlas_proto as pb;
use trogon_atlas_types::{
    compile_with_imports, CompileLimits, ProtoPackagePrefix, SourceBundle, TypeLibrarySource,
};

use crate::{
    client::Client,
    compat::{self, Capability},
};

const TYPE_LIBRARY_MESSAGE: &str = "trogonatlas.eventmodel.v1alpha1.TypeLibrary";

/// The namespaces whose live type libraries a payload may name, and the
/// libraries written alongside it.
#[derive(Debug, Clone, Default)]
pub struct TypeScope {
    namespaces: BTreeSet<String>,
    pending: BTreeMap<(String, String), pb::TypeLibrary>,
}

impl TypeScope {
    pub fn add_namespace(&mut self, namespace: impl Into<String>) {
        self.namespaces.insert(namespace.into());
    }

    /// A library written alongside the payload. Its source stands in for the
    /// live version of the same slug; of several versions, the highest wins.
    pub fn add_pending(&mut self, library: pb::TypeLibrary) {
        let Some(id) = library.id.as_ref() else {
            return;
        };
        self.namespaces.insert(id.namespace.clone());
        let key = (id.namespace.clone(), id.slug.clone());
        let newer = self
            .pending
            .get(&key)
            .and_then(|existing| existing.id.as_ref())
            .is_none_or(|existing| existing.version <= id.version);
        if newer {
            self.pending.insert(key, library);
        }
    }

    pub fn merge(&mut self, other: Self) {
        self.namespaces.extend(other.namespaces);
        for library in other.pending.into_values() {
            self.add_pending(library);
        }
    }

    /// The namespaces and type libraries a proto3-JSON payload names: every
    /// `id.namespace` anywhere in it, and every `typeLibrary` body.
    #[must_use]
    pub fn of_json(value: &Value) -> Self {
        let mut scope = Self::default();
        scope.collect(value);
        scope
    }

    fn collect(&mut self, value: &Value) {
        match value {
            Value::Object(map) => {
                for (key, child) in map {
                    match (key.as_str(), child) {
                        ("id", Value::Object(id)) => {
                            if let Some(namespace) = id.get("namespace").and_then(Value::as_str) {
                                self.add_namespace(namespace);
                            }
                        }
                        ("typeLibrary" | "type_library", Value::Object(_)) => {
                            if let Ok(library) = TranscodePool::builtin().and_then(|pool| {
                                pool.message_from_json_value(TYPE_LIBRARY_MESSAGE, child)
                            }) {
                                self.add_pending(library);
                            }
                        }
                        _ => {}
                    }
                    self.collect(child);
                }
            }
            Value::Array(items) => items.iter().for_each(|item| self.collect(item)),
            _ => {}
        }
    }

    fn pending_in<'a>(&'a self, namespace: &'a str) -> impl Iterator<Item = &'a pb::TypeLibrary> {
        self.pending
            .iter()
            .filter(move |((ns, _), _)| ns == namespace)
            .map(|(_, library)| library)
    }
}

/// The built-in descriptors with the tenant types `scope` reaches layered
/// on. A server that does not advertise type libraries has none to offer,
/// so the built-ins alone come back. A pending library that does not compile
/// leaves its live version in place: the server owns reporting why.
pub async fn transcode_pool(client: &mut Client, scope: &TypeScope) -> Result<TranscodePool> {
    let builtin = TranscodePool::builtin()?;
    if scope.namespaces.is_empty() || !compat::advertises(client, Capability::TypeLibraries).await?
    {
        return Ok(builtin);
    }
    let mut files = TenantFiles::default();
    for namespace in &scope.namespaces {
        let served = client
            .get_type_library_descriptor_set(pb::GetTypeLibraryDescriptorSetRequest {
                namespace: namespace.clone(),
            })
            .await
            .with_context(|| format!("GetTypeLibraryDescriptorSet for namespace {namespace:?}"))?
            .into_inner();
        let served = FileDescriptorSet::decode(served.file_descriptor_set.as_slice())
            .with_context(|| format!("decoding the type libraries of namespace {namespace:?}"))?;
        let pending: Vec<TypeLibrarySource> =
            scope.pending_in(namespace).filter_map(source_of).collect();
        let imports = FileDescriptorSet {
            file: served
                .file
                .iter()
                .filter(|file| !pending.iter().any(|s| s.prefix().owns(file.package())))
                .cloned()
                .collect(),
        };
        let mut replaced = Vec::new();
        for (i, library) in pending.iter().enumerate() {
            let others: Vec<TypeLibrarySource> = pending
                .iter()
                .enumerate()
                .filter(|(j, _)| *j != i)
                .map(|(_, other)| other.clone())
                .collect();
            if let Ok(compiled) = compile_with_imports(library, &others, &imports) {
                files.extend(namespace, compiled.file_descriptor_set().file.clone())?;
                replaced.push(library.prefix().clone());
            }
        }
        files.extend(
            namespace,
            served
                .file
                .into_iter()
                .filter(|file| !replaced.iter().any(|prefix| prefix.owns(file.package()))),
        )?;
    }
    Ok(builtin.with_files(files.by_name.into_values().map(|(_, file)| file))?)
}

fn source_of(library: &pb::TypeLibrary) -> Option<TypeLibrarySource> {
    let prefix = ProtoPackagePrefix::parse(&library.id.as_ref()?.slug).ok()?;
    let bundle = SourceBundle::from_proto(&library.files, &CompileLimits::default()).ok()?;
    Some(TypeLibrarySource::new(prefix, bundle))
}

#[derive(Default)]
struct TenantFiles {
    by_name: BTreeMap<String, (String, FileDescriptorProto)>,
}

impl TenantFiles {
    fn extend(
        &mut self,
        namespace: &str,
        files: impl IntoIterator<Item = FileDescriptorProto>,
    ) -> Result<()> {
        for file in files {
            let name = file.name().to_owned();
            match self.by_name.get(&name) {
                Some((owner, existing)) if *existing != file => {
                    crate::invalid_input!(
                        "namespaces {owner:?} and {namespace:?} both declare {name} with different content; transcode them separately"
                    );
                }
                Some(_) => {}
                None => {
                    self.by_name.insert(name, (namespace.to_owned(), file));
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use serde_json::json;

    use super::*;

    #[test]
    fn json_scope_finds_namespaces_and_libraries() {
        let scope = TypeScope::of_json(&json!({
            "ops": [
                {"put": {"entity": {"typeLibrary": {
                    "id": {"namespace": "shop", "slug": "shop.orders", "version": "2"},
                    "files": [{"path": "shop/orders/v1/orders.proto", "content": "syntax = \"proto3\";"}]
                }}}},
                {"put": {"entity": {"event": {"id": {"namespace": "billing", "slug": "paid", "version": "1"}}}}}
            ]
        }));
        assert_eq!(
            scope.namespaces.iter().collect::<Vec<_>>(),
            vec!["billing", "shop"]
        );
        assert_eq!(scope.pending_in("shop").count(), 1);
        assert_eq!(scope.pending_in("billing").count(), 0);
    }

    #[test]
    fn the_highest_pending_version_of_a_library_wins() {
        let library = |version| pb::TypeLibrary {
            id: Some(pb::Id {
                namespace: "shop".into(),
                slug: "shop.orders".into(),
                version,
            }),
            ..Default::default()
        };
        let mut scope = TypeScope::default();
        scope.add_pending(library(2));
        scope.add_pending(library(1));
        let kept: Vec<u64> = scope
            .pending_in("shop")
            .map(|l| l.id.as_ref().unwrap().version)
            .collect();
        assert_eq!(kept, vec![2]);
    }
}
