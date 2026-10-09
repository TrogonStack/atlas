//! Tenant type libraries: protobuf source text compiled in memory, never
//! persisted in compiled form.

mod bundle;
mod compat;
mod compile;
mod dependencies;
mod package;
mod path;
mod pools;
mod runner;
mod type_url;

pub use bundle::{BundleError, CompileLimits, SourceBundle, SourceFile, TypeLibraryDigest};
pub use compat::{breaking_changes, BreakingChange};
pub use compile::{
    compile, compile_with_imports, CompileError, CompiledLibrary, Diagnostic, TypeLibrarySource,
};
pub use dependencies::{resolve_dependencies, DependencyError, LibraryNode};
pub use package::{ProtoPackagePrefix, ProtoPackagePrefixError};
pub use path::{ProtoPath, ProtoPathError};
pub use pools::{PoolError, PoolMember, PoolScope, TypePools};
pub use runner::CompileRunner;
pub use type_url::{TypeUrl, TypeUrlError};
