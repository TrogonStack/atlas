use std::fmt;

use sha2::{Digest, Sha256};
use trogon_atlas_proto as pb;

use crate::path::{ProtoPath, ProtoPathError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompileLimits {
    pub max_files: usize,
    pub max_file_bytes: usize,
    pub max_total_bytes: usize,
    pub max_dependencies: usize,
    pub max_dependency_depth: usize,
}

impl Default for CompileLimits {
    fn default() -> Self {
        Self {
            max_files: 64,
            max_file_bytes: 256 * 1024,
            max_total_bytes: 1024 * 1024,
            max_dependencies: 16,
            max_dependency_depth: 8,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BundleError {
    #[error("a type library needs at least one file")]
    Empty,
    #[error(transparent)]
    Path(#[from] ProtoPathError),
    #[error("file {0} appears more than once")]
    DuplicatePath(ProtoPath),
    #[error("{count} files exceed the limit of {max}")]
    TooManyFiles { count: usize, max: usize },
    #[error("file {path} is {bytes} bytes, over the limit of {max}")]
    FileTooLarge {
        path: ProtoPath,
        bytes: usize,
        max: usize,
    },
    #[error("files total {bytes} bytes, over the limit of {max}")]
    BundleTooLarge { bytes: usize, max: usize },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFile {
    path: ProtoPath,
    content: String,
}

impl SourceFile {
    pub fn new(path: ProtoPath, content: impl Into<String>) -> Self {
        Self {
            path,
            content: content.into(),
        }
    }

    pub fn path(&self) -> &ProtoPath {
        &self.path
    }

    pub fn content(&self) -> &str {
        &self.content
    }
}

/// The source text of one type library, ordered by path so equal content
/// always yields the same digest and the same compile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceBundle {
    files: Vec<SourceFile>,
}

impl SourceBundle {
    pub fn new(
        files: impl IntoIterator<Item = SourceFile>,
        limits: &CompileLimits,
    ) -> Result<Self, BundleError> {
        let mut files: Vec<SourceFile> = files.into_iter().collect();
        if files.is_empty() {
            return Err(BundleError::Empty);
        }
        if files.len() > limits.max_files {
            return Err(BundleError::TooManyFiles {
                count: files.len(),
                max: limits.max_files,
            });
        }
        files.sort_by(|a, b| a.path.cmp(&b.path));
        if let Some(pair) = files.windows(2).find(|w| w[0].path == w[1].path) {
            return Err(BundleError::DuplicatePath(pair[0].path.clone()));
        }
        let mut total = 0usize;
        for file in &files {
            let bytes = file.content.len();
            if bytes > limits.max_file_bytes {
                return Err(BundleError::FileTooLarge {
                    path: file.path.clone(),
                    bytes,
                    max: limits.max_file_bytes,
                });
            }
            total += bytes;
        }
        if total > limits.max_total_bytes {
            return Err(BundleError::BundleTooLarge {
                bytes: total,
                max: limits.max_total_bytes,
            });
        }
        Ok(Self { files })
    }

    pub fn from_proto(
        files: &[pb::ProtoSourceFile],
        limits: &CompileLimits,
    ) -> Result<Self, BundleError> {
        let files = files
            .iter()
            .map(|f| {
                Ok(SourceFile::new(
                    ProtoPath::parse(&f.path)?,
                    f.content.clone(),
                ))
            })
            .collect::<Result<Vec<_>, BundleError>>()?;
        Self::new(files, limits)
    }

    pub fn files(&self) -> &[SourceFile] {
        &self.files
    }

    pub fn get(&self, path: &str) -> Option<&SourceFile> {
        self.files
            .binary_search_by(|f| f.path.as_str().cmp(path))
            .ok()
            .map(|i| &self.files[i])
    }

    pub fn digest(&self) -> TypeLibraryDigest {
        let mut hasher = Sha256::new();
        hasher.update(b"trogonatlas.type_library.source.v1\0");
        for file in &self.files {
            for part in [file.path.as_str().as_bytes(), file.content.as_bytes()] {
                hasher.update((part.len() as u64).to_be_bytes());
                hasher.update(part);
            }
        }
        TypeLibraryDigest(hasher.finalize().into())
    }
}

/// SHA-256 over a [`SourceBundle`]'s canonical encoding: the identity of a
/// library's source independent of the order its files arrived in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TypeLibraryDigest([u8; 32]);

impl TypeLibraryDigest {
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Display for TypeLibraryDigest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("sha256:")?;
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn file(path: &str, content: &str) -> SourceFile {
        SourceFile::new(ProtoPath::parse(path).unwrap(), content)
    }

    #[test]
    fn digest_ignores_arrival_order() {
        let limits = CompileLimits::default();
        let a =
            SourceBundle::new([file("a/a.proto", "x"), file("b/b.proto", "y")], &limits).unwrap();
        let b =
            SourceBundle::new([file("b/b.proto", "y"), file("a/a.proto", "x")], &limits).unwrap();
        assert_eq!(a.digest(), b.digest());
    }

    #[test]
    fn digest_separates_path_from_content() {
        let limits = CompileLimits::default();
        let a = SourceBundle::new([file("a/ab.proto", "c")], &limits).unwrap();
        let b = SourceBundle::new([file("a/a.proto", "bc")], &limits).unwrap();
        assert_ne!(a.digest(), b.digest());
        assert!(a.digest().to_string().starts_with("sha256:"));
        assert_eq!(a.digest().to_string().len(), "sha256:".len() + 64);
    }

    #[test]
    fn enforces_every_size_limit() {
        let limits = CompileLimits {
            max_files: 2,
            max_file_bytes: 4,
            max_total_bytes: 6,
            ..CompileLimits::default()
        };
        assert_eq!(SourceBundle::new([], &limits), Err(BundleError::Empty));
        assert_eq!(
            SourceBundle::new(
                [
                    file("a.proto", ""),
                    file("b.proto", ""),
                    file("c.proto", "")
                ],
                &limits
            ),
            Err(BundleError::TooManyFiles { count: 3, max: 2 })
        );
        assert!(matches!(
            SourceBundle::new([file("a.proto", "12345")], &limits),
            Err(BundleError::FileTooLarge {
                bytes: 5,
                max: 4,
                ..
            })
        ));
        assert_eq!(
            SourceBundle::new([file("a.proto", "1234"), file("b.proto", "123")], &limits),
            Err(BundleError::BundleTooLarge { bytes: 7, max: 6 })
        );
        assert!(matches!(
            SourceBundle::new([file("a.proto", ""), file("a.proto", "")], &limits),
            Err(BundleError::DuplicatePath(_))
        ));
    }

    #[test]
    fn from_proto_rejects_bad_paths() {
        let err = SourceBundle::from_proto(
            &[pb::ProtoSourceFile {
                path: "../escape.proto".into(),
                content: String::new(),
            }],
            &CompileLimits::default(),
        )
        .unwrap_err();
        assert!(matches!(
            err,
            BundleError::Path(ProtoPathError::NotRelative(_))
        ));
    }
}
