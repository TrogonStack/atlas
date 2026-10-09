use std::{collections::HashMap, fmt, sync::Arc};

use miette::Diagnostic as _;
use prost_types::{FileDescriptorProto, FileDescriptorSet};
use protox::file::{
    ChainFileResolver, DescriptorSetFileResolver, File, FileResolver, GoogleFileResolver,
};

use crate::{
    bundle::{SourceBundle, SourceFile, TypeLibraryDigest},
    compat::{breaking_changes, BreakingChange},
    package::ProtoPackagePrefix,
    path::{is_atlas_import, is_reserved_import},
};

const PACKAGE_FIELD: i32 = 2;
const DEPENDENCY_FIELD: i32 = 3;

/// One library's source together with the package prefix it owns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeLibrarySource {
    prefix: ProtoPackagePrefix,
    bundle: Arc<SourceBundle>,
}

impl TypeLibrarySource {
    pub fn new(prefix: ProtoPackagePrefix, bundle: SourceBundle) -> Self {
        Self {
            prefix,
            bundle: Arc::new(bundle),
        }
    }

    pub fn prefix(&self) -> &ProtoPackagePrefix {
        &self.prefix
    }

    pub fn bundle(&self) -> &SourceBundle {
        &self.bundle
    }

    pub fn digest(&self) -> TypeLibraryDigest {
        self.bundle.digest()
    }
}

/// A source location a tenant can act on. Lines and columns are 1-based;
/// zero means the compiler could not attribute the problem to a position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub path: String,
    pub line: u32,
    pub column: u32,
    pub message: String,
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{}:{}: {}",
            self.path, self.line, self.column, self.message
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CompileError {
    #[error("type library does not compile: {}", join(.0))]
    Diagnostics(Vec<Diagnostic>),
    #[error("type library compile did not finish within {0:?}")]
    TimedOut(std::time::Duration),
    #[error("type library compiler is unavailable")]
    Unavailable,
}

fn join(diagnostics: &[Diagnostic]) -> String {
    diagnostics
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ")
}

/// The descriptors compiled from one library's own files. Dependency files
/// are left out so pools can stack libraries without duplicating them.
#[derive(Debug, Clone, PartialEq)]
pub struct CompiledLibrary {
    digest: TypeLibraryDigest,
    prefix: ProtoPackagePrefix,
    files: FileDescriptorSet,
}

impl CompiledLibrary {
    pub fn digest(&self) -> TypeLibraryDigest {
        self.digest
    }

    pub fn prefix(&self) -> &ProtoPackagePrefix {
        &self.prefix
    }

    pub fn file_descriptor_set(&self) -> &FileDescriptorSet {
        &self.files
    }

    pub fn breaking_changes_since(&self, previous: &Self) -> Vec<BreakingChange> {
        breaking_changes(&previous.files, &self.files)
    }

    pub fn message_names(&self) -> Vec<String> {
        let mut names = Vec::new();
        for file in &self.files.file {
            let package = file.package();
            for message in &file.message_type {
                collect_message_names(package, message, &mut names);
            }
        }
        names
    }
}

fn collect_message_names(
    scope: &str,
    message: &prost_types::DescriptorProto,
    out: &mut Vec<String>,
) {
    let name = if scope.is_empty() {
        message.name().to_owned()
    } else {
        format!("{scope}.{}", message.name())
    };
    for nested in &message.nested_type {
        if !nested
            .options
            .as_ref()
            .is_some_and(prost_types::MessageOptions::map_entry)
        {
            collect_message_names(&name, nested, out);
        }
    }
    out.push(name);
}

/// Compiles `library` against the files of `dependencies` and the Google
/// well-known types. Nothing is read from the filesystem or the network.
pub fn compile(
    library: &TypeLibrarySource,
    dependencies: &[TypeLibrarySource],
) -> Result<CompiledLibrary, CompileError> {
    compile_with_imports(library, dependencies, &FileDescriptorSet::default())
}

/// As [`compile`], also resolving imports from `imports`: files compiled
/// earlier whose source the caller does not hold. Sources in `library` and
/// `dependencies` win over a file of the same name in `imports`.
pub fn compile_with_imports(
    library: &TypeLibrarySource,
    dependencies: &[TypeLibrarySource],
    imports: &FileDescriptorSet,
) -> Result<CompiledLibrary, CompileError> {
    let mut diagnostics = precheck(library, dependencies);
    if !diagnostics.is_empty() {
        diagnostics.sort_by(|a, b| (&a.path, a.line, a.column).cmp(&(&b.path, b.line, b.column)));
        return Err(CompileError::Diagnostics(diagnostics));
    }

    let sources: Vec<Arc<SourceBundle>> = std::iter::once(library)
        .chain(dependencies)
        .map(|l| Arc::clone(&l.bundle))
        .collect();
    let mut chain = ChainFileResolver::new();
    chain.add(InMemoryResolver(sources.clone()));
    chain.add(GoogleFileResolver::new());
    chain.add(DescriptorSetFileResolver::new(imports.clone()));
    let mut compiler = protox::Compiler::with_file_resolver(chain);
    compiler.include_imports(false);
    compiler.include_source_info(false);
    for file in library.bundle.files() {
        if let Err(err) = compiler.open_file(file.path().as_str()) {
            return Err(CompileError::Diagnostics(vec![from_protox(&err, &sources)]));
        }
    }

    Ok(CompiledLibrary {
        digest: library.digest(),
        prefix: library.prefix.clone(),
        files: compiler.file_descriptor_set(),
    })
}

fn precheck(library: &TypeLibrarySource, dependencies: &[TypeLibrarySource]) -> Vec<Diagnostic> {
    let mut provided_by: HashMap<&str, &ProtoPackagePrefix> = HashMap::new();
    for dep in dependencies {
        for file in dep.bundle.files() {
            provided_by.insert(file.path().as_str(), &dep.prefix);
        }
    }

    let mut out = Vec::new();
    for file in library.bundle.files() {
        let path = file.path().as_str();
        if let Some(owner) = provided_by.get(path) {
            out.push(Diagnostic {
                path: path.to_owned(),
                line: 0,
                column: 0,
                message: format!("path is already provided by dependency {owner}"),
            });
        }
        let parsed = match File::from_source(path, file.content()) {
            Ok(parsed) => parsed,
            Err(err) => {
                out.push(from_protox(&err, std::slice::from_ref(&library.bundle)));
                continue;
            }
        };
        let descriptor = parsed.file_descriptor_proto();
        let package = descriptor.package();
        if !library.prefix.owns(package) {
            let (line, column) = span_start(descriptor, &[PACKAGE_FIELD]);
            out.push(Diagnostic {
                path: path.to_owned(),
                line,
                column,
                message: if package.is_empty() {
                    format!("file must declare a package under {}", library.prefix)
                } else {
                    format!(
                        "package {package} is outside the library's prefix {}",
                        library.prefix
                    )
                },
            });
        }
        for (i, import) in descriptor.dependency.iter().enumerate() {
            if is_atlas_import(import) {
                let (line, column) = span_start(
                    descriptor,
                    &[DEPENDENCY_FIELD, i32::try_from(i).unwrap_or(0)],
                );
                out.push(Diagnostic {
                    path: path.to_owned(),
                    line,
                    column,
                    message: format!("import {import:?} is reserved for the atlas itself"),
                });
            }
        }
    }
    out
}

fn span_start(descriptor: &FileDescriptorProto, path: &[i32]) -> (u32, u32) {
    descriptor
        .source_code_info
        .as_ref()
        .and_then(|info| info.location.iter().find(|l| l.path == path))
        .and_then(|l| Some((l.span.first()?, l.span.get(1)?)))
        .map_or((0, 0), |(&line, &column)| {
            (
                u32::try_from(line + 1).unwrap_or(0),
                u32::try_from(column + 1).unwrap_or(0),
            )
        })
}

fn from_protox(err: &protox::Error, sources: &[Arc<SourceBundle>]) -> Diagnostic {
    let path = err.file().unwrap_or_default().to_owned();
    let offset = err
        .labels()
        .and_then(|mut labels| labels.next())
        .map(|l| l.offset());
    let source = sources
        .iter()
        .find_map(|b| b.get(&path))
        .map(SourceFile::content);
    let (line, column) = match (offset, source) {
        (Some(offset), Some(source)) => line_column(source, offset),
        _ => (0, 0),
    };
    Diagnostic {
        path,
        line,
        column,
        message: err.to_string(),
    }
}

fn line_column(source: &str, offset: usize) -> (u32, u32) {
    let before = &source.as_bytes()[..offset.min(source.len())];
    let line = before.split(|&b| b == b'\n').count();
    let line_start = before
        .iter()
        .rposition(|&b| b == b'\n')
        .map_or(0, |i| i + 1);
    let column = String::from_utf8_lossy(&before[line_start..])
        .chars()
        .count()
        + 1;
    (
        u32::try_from(line).unwrap_or(u32::MAX),
        u32::try_from(column).unwrap_or(u32::MAX),
    )
}

struct InMemoryResolver(Vec<Arc<SourceBundle>>);

impl FileResolver for InMemoryResolver {
    fn open_file(&self, name: &str) -> Result<File, protox::Error> {
        if is_reserved_import(name) {
            return Err(protox::Error::file_not_found(name));
        }
        match self.0.iter().find_map(|bundle| bundle.get(name)) {
            Some(file) => File::from_source(name, file.content()),
            None => Err(protox::Error::file_not_found(name)),
        }
    }
}
