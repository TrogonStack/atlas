use std::{collections::HashMap, fmt, hash::Hash};

use crate::{bundle::CompileLimits, compile::TypeLibrarySource};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DependencyError<K: fmt::Debug + fmt::Display> {
    #[error("dependency {0} does not exist")]
    Missing(K),
    #[error("dependency cycle through {0}")]
    Cycle(K),
    #[error("{count} transitive dependencies exceed the limit of {max}")]
    TooMany { count: usize, max: usize },
    #[error("dependency chain of depth {depth} exceeds the limit of {max}")]
    TooDeep { depth: usize, max: usize },
}

/// A library as the dependency resolver sees it: its source and the keys of
/// the libraries it imports from.
#[derive(Debug, Clone)]
pub struct LibraryNode<K> {
    pub source: TypeLibrarySource,
    pub dependencies: Vec<K>,
}

/// Every library reachable from `direct`, dependencies before dependents,
/// within the configured count and depth limits.
pub fn resolve_dependencies<K, F>(
    root: &K,
    direct: &[K],
    mut lookup: F,
    limits: &CompileLimits,
) -> Result<Vec<(K, TypeLibrarySource)>, DependencyError<K>>
where
    K: Clone + Eq + Hash + Ord + fmt::Debug + fmt::Display,
    F: FnMut(&K) -> Option<LibraryNode<K>>,
{
    let mut walk = Walk {
        root,
        lookup: &mut lookup,
        limits,
        depth: HashMap::new(),
        on_stack: Vec::new(),
        order: Vec::new(),
    };
    let mut direct = direct.to_vec();
    direct.sort();
    direct.dedup();
    for key in &direct {
        walk.visit(key)?;
    }
    Ok(walk.order)
}

struct Walk<'a, K, F> {
    root: &'a K,
    lookup: &'a mut F,
    limits: &'a CompileLimits,
    depth: HashMap<K, usize>,
    on_stack: Vec<K>,
    order: Vec<(K, TypeLibrarySource)>,
}

impl<K, F> Walk<'_, K, F>
where
    K: Clone + Eq + Hash + Ord + fmt::Debug + fmt::Display,
    F: FnMut(&K) -> Option<LibraryNode<K>>,
{
    fn visit(&mut self, key: &K) -> Result<usize, DependencyError<K>> {
        if key == self.root || self.on_stack.contains(key) {
            return Err(DependencyError::Cycle(key.clone()));
        }
        if let Some(&depth) = self.depth.get(key) {
            return Ok(depth);
        }
        let node = (self.lookup)(key).ok_or_else(|| DependencyError::Missing(key.clone()))?;
        self.on_stack.push(key.clone());
        let mut children = node.dependencies;
        children.sort();
        children.dedup();
        let mut depth = 1;
        for child in &children {
            depth = depth.max(self.visit(child)? + 1);
        }
        self.on_stack.pop();
        if depth > self.limits.max_dependency_depth {
            return Err(DependencyError::TooDeep {
                depth,
                max: self.limits.max_dependency_depth,
            });
        }
        self.depth.insert(key.clone(), depth);
        self.order.push((key.clone(), node.source));
        if self.order.len() > self.limits.max_dependencies {
            return Err(DependencyError::TooMany {
                count: self.order.len(),
                max: self.limits.max_dependencies,
            });
        }
        Ok(depth)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::{
        bundle::{SourceBundle, SourceFile},
        package::ProtoPackagePrefix,
        path::ProtoPath,
    };

    fn source(slug: &str) -> TypeLibrarySource {
        TypeLibrarySource::new(
            ProtoPackagePrefix::parse(slug).unwrap(),
            SourceBundle::new(
                [SourceFile::new(
                    ProtoPath::parse(&format!("{slug}.proto")).unwrap(),
                    "",
                )],
                &CompileLimits::default(),
            )
            .unwrap(),
        )
    }

    fn graph(edges: &[(&str, &[&str])]) -> HashMap<String, Vec<String>> {
        edges
            .iter()
            .map(|(k, v)| ((*k).to_owned(), v.iter().map(|s| (*s).to_owned()).collect()))
            .collect()
    }

    fn resolve(
        g: &HashMap<String, Vec<String>>,
        root: &str,
        limits: &CompileLimits,
    ) -> Result<Vec<String>, DependencyError<String>> {
        let direct = g.get(root).cloned().unwrap_or_default();
        resolve_dependencies(
            &root.to_owned(),
            &direct,
            |k| {
                g.get(k).map(|deps| LibraryNode {
                    source: source(k),
                    dependencies: deps.clone(),
                })
            },
            limits,
        )
        .map(|v| v.into_iter().map(|(k, _)| k).collect())
    }

    #[test]
    fn orders_dependencies_before_dependents() {
        let g = graph(&[("app", &["b", "a"]), ("b", &["a"]), ("a", &[])]);
        assert_eq!(
            resolve(&g, "app", &CompileLimits::default()).unwrap(),
            vec!["a", "b"]
        );
    }

    #[test]
    fn rejects_missing_and_cyclic_dependencies() {
        let g = graph(&[("app", &["a"]), ("a", &["ghost"])]);
        assert_eq!(
            resolve(&g, "app", &CompileLimits::default()),
            Err(DependencyError::Missing("ghost".into()))
        );
        let g = graph(&[("app", &["a"]), ("a", &["b"]), ("b", &["a"])]);
        assert_eq!(
            resolve(&g, "app", &CompileLimits::default()),
            Err(DependencyError::Cycle("a".into()))
        );
        let g = graph(&[("app", &["a"]), ("a", &["app"])]);
        assert_eq!(
            resolve(&g, "app", &CompileLimits::default()),
            Err(DependencyError::Cycle("app".into()))
        );
    }

    #[test]
    fn enforces_count_and_depth_limits() {
        let limits = CompileLimits {
            max_dependencies: 2,
            max_dependency_depth: 2,
            ..CompileLimits::default()
        };
        let g = graph(&[
            ("app", &["a", "b", "c"]),
            ("a", &[]),
            ("b", &[]),
            ("c", &[]),
        ]);
        assert_eq!(
            resolve(&g, "app", &limits),
            Err(DependencyError::TooMany { count: 3, max: 2 })
        );
        let g = graph(&[("app", &["a"]), ("a", &["b"]), ("b", &["c"]), ("c", &[])]);
        assert_eq!(
            resolve(&g, "app", &limits),
            Err(DependencyError::TooDeep { depth: 3, max: 2 })
        );
    }
}
