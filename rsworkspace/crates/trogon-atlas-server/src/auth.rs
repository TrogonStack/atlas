use std::{
    collections::{HashMap, HashSet},
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use subtle::ConstantTimeEq;
use tonic::{Request, Status};
use tower::{Layer, Service};

// ---------------------------------------------------------------------------
// Role
// ---------------------------------------------------------------------------

/// Authorization level. Roles are ordered: Reader < Writer < Admin.
/// A principal satisfies a required role when its own role is >= the required
/// one (numeric comparison via `PartialOrd` and `Ord`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Reader = 0,
    Writer = 1,
    Admin = 2,
}

// ---------------------------------------------------------------------------
// PrincipalKind
// ---------------------------------------------------------------------------

/// Whether a principal is an owner acting as itself, or a delegate acting on
/// an owner's behalf.
///
/// Exists for exactly one question: does this principal hold rights over an
/// owned branch (`crate::ownership::OwnedBranchName`)? `User` holds them by
/// being the owner (matched by name); `Agent` holds them only when the owner
/// has delegated to it (matched by `parent`, the same field
/// `crate::ownership::Visibility` already consults for namespace tenancy).
/// Delegation is per owner, never per namespace or per branch, which is why
/// `Principal::authorizes_owner` takes nothing more specific than an
/// `OwnerId`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PrincipalKind {
    #[default]
    User,
    Agent,
}

// ---------------------------------------------------------------------------
// Principal
// ---------------------------------------------------------------------------

/// Authenticated caller identity resolved from a bearer token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Principal {
    pub name: Arc<str>,
    pub role: Role,
    pub kind: PrincipalKind,
    /// True only for the synthetic principal `BearerAuth` inserts when
    /// `--insecure-allow-anonymous` is active (`TokenRegistry::anonymous_admin`).
    /// That principal carries `Role::Admin` so normal RBAC checks pass for
    /// every RPC, but it is not an authenticated Admin: it exists so an
    /// insecure dev stack does not require a token file at all. Callers that
    /// need to distinguish "really is Admin" from "RBAC is switched off"
    /// (e.g. the baseline write protection escape hatch) must check this
    /// flag, not just `role`.
    pub is_anonymous: bool,
    /// The namespaces this principal may write. Empty (the default, and what
    /// every registry written before this field parses into) means
    /// unrestricted. Reads are never scoped: see
    /// `crate::scope::NamespaceScope`.
    ///
    /// The role decides *whether* a caller may write; this decides *where*.
    /// It cannot be enforced next to the role check, because the gRPC path
    /// the `AuthzLayer` keys on carries no namespace -- only the request body
    /// does. Enforcement therefore lives in the handlers, at the boundary
    /// that already resolves branch context.
    pub namespaces: crate::scope::NamespaceScope,
    /// The ownership node this principal is bound to, if any.
    ///
    /// `None` means no owner boundary: the caller reads and writes across
    /// every namespace, subject only to `role` and `namespaces`. That is
    /// what every registry written before this field parses into, and what
    /// keeps the feature opt-in.
    ///
    /// `Some(owner)` binds the caller to one subtree of the namespace
    /// registry. Unlike `namespaces`, which only constrains writes, this
    /// constrains reads too, because it is a tenancy boundary rather than a
    /// blast-radius limiter. The two compose: `parent` decides which
    /// namespaces exist as far as this caller is concerned, and `namespaces`
    /// then decides which of those it may write.
    pub parent: Option<trogon_atlas_core::OwnerId>,
}

impl Principal {
    /// Whether this principal holds rights over an owned branch belonging
    /// to `owner`: `crate::ownership::OwnedBranchName::owner`.
    ///
    /// A `User` is the owner acting as itself, so it matches by name: a
    /// principal literally named `acme` is `acme`. An `Agent` holds no
    /// identity of its own here; it matches only when `acme` delegated to it,
    /// recorded the same way `parent` already records namespace tenancy.
    /// This is why the negative case holds without any extra bookkeeping: an
    /// agent `parent = Some(alice)` never matches `owner = bob`, no matter
    /// what the agent is named.
    #[must_use]
    pub fn authorizes_owner(&self, owner: &trogon_atlas_core::OwnerId) -> bool {
        match self.kind {
            PrincipalKind::User => self.name.as_ref() == owner.as_str(),
            PrincipalKind::Agent => self.parent.as_ref() == Some(owner),
        }
    }
}

// ---------------------------------------------------------------------------
// Method map
// ---------------------------------------------------------------------------

/// Return the minimum role required to call `grpc_path`, or `None` when the
/// path is not in the map. An unmapped path is denied (`PERMISSION_DENIED`).
///
/// The full path format is `/<package>.<Service>/<Method>`, matching what
/// tonic places in `http::Request::uri().path()`.
#[must_use]
#[allow(clippy::match_same_arms)]
pub fn required_role(grpc_path: &str) -> Option<Role> {
    Some(match grpc_path {
        // Bootstrap
        "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/GetServerInfo" => Role::Reader,
        "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/ListNamespaces" => Role::Reader,
        // A principal learning its own identity is never more privileged
        // than the identity itself, so this is Reader regardless of what
        // role the caller actually holds.
        "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/WhoAmI" => Role::Reader,
        // A dry-run compile writes nothing, so checking a library before
        // writing it needs no more than reading the libraries it depends on.
        "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/CompileTypeLibrary" => Role::Reader,
        "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/ResolveType" => Role::Reader,
        "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/GetTypeLibraryDescriptorSet" => {
            Role::Reader
        }
        // Claiming a name is a write, and one a tenant must be able to do
        // for itself. Moving a namespace between owners is not: it is the
        // only operation that crosses a tenancy boundary, so no principal
        // bound inside one may perform it.
        "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/RegisterNamespace" => Role::Writer,
        "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/MoveNamespace" => Role::Admin,
        "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/GetSnapshotId" => Role::Reader,

        // Direct lookup
        "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/GetEntity" => Role::Reader,
        "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/BatchGetEntities" => Role::Reader,

        // Discovery
        "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/ListEntities" => Role::Reader,
        "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/SearchEntities" => Role::Reader,

        // Versioning
        "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/ListVersions" => Role::Reader,
        "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/GetLatestVersion" => Role::Reader,
        "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/GetSupersessionChain" => {
            Role::Reader
        }

        // Graph traversal
        "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/GetIncomingReferences" => {
            Role::Reader
        }
        "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/GetOutgoingReferences" => {
            Role::Reader
        }
        "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/GetImpact" => Role::Reader,

        // Denormalized projections
        "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/GetSliceProjection" => Role::Reader,
        "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/GetStoryboardProjection" => {
            Role::Reader
        }
        "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/GetEventModelProjection" => {
            Role::Reader
        }
        "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/ExtractSubgraph" => Role::Reader,

        // Structural comparison
        "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/DiffEntities" => Role::Reader,

        // Mutation
        "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/PutEntity" => Role::Writer,
        // DeleteEntity is a single-entity targeted delete and requires Writer
        // (not Admin), matching its scope. Only the bulk/destructive
        // DeleteByQuery path requires Admin. This keeps the common "delete one
        // stale entity" flow accessible to writers.
        "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/DeleteEntity" => Role::Writer,
        "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/BatchMutate" => Role::Writer,

        // Change feed
        "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/ListChanges" => Role::Reader,
        "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/StreamChanges" => Role::Reader,

        // Changeset log. Reading history is a read: it exposes nothing a
        // Reader could not already see by listing entities, plus the author
        // that produced each write.
        "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/ListChangesets" => Role::Reader,
        "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/GetChangeset" => Role::Reader,
        "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/GetEntityHistory" => Role::Reader,
        // Revert writes an inverse batch through the ordinary write path, so
        // it is exactly as privileged as the writes it undoes: its targets
        // are a fixed list read off a recorded changeset, not a query the
        // caller composes, which is what makes DeleteByQuery Admin.
        "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/RevertChangeset" => Role::Writer,

        // Validation (read-only, no mutation)
        "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/ValidateEventModel" => Role::Reader,
        "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/ListValidationRules" => {
            Role::Reader
        }
        "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/ValidateProject" => Role::Reader,

        // Bulk destructive delete: Admin only
        "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/DeleteByQuery" => Role::Admin,

        // Static catalog
        "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/ListEntityKinds" => Role::Reader,
        "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/ListEntitiesByDomain" => {
            Role::Reader
        }

        // AI-driven analysis (spend LLM tokens): Writer
        "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/InferDataFlow" => Role::Writer,
        "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/CheckInformationCompleteness" => {
            Role::Writer
        }

        // Bulk reference retargeting (rewrites referrers): Writer
        "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/RetargetReferences" => Role::Writer,

        // Branching (Phase 1: Isolation): creating/deleting a branch is a
        // mutation of server state, so both require Writer. Listing branches
        // is a read: Readers observe the work agents do on branches, and a
        // Reader could already diff any branch it can name.
        "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/CreateBranch" => Role::Writer,
        "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/ListBranches" => Role::Reader,
        "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/DeleteBranch" => Role::Writer,

        // Branching (Phase 2: Review and merge). Diffing is read-only
        // (Reader); merging, updating, and resolving conflicts mutate
        // branch and/or baseline state (Writer), matching the lifecycle
        // RPCs above.
        "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/DiffBranch" => Role::Reader,
        "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/MergeBranch" => Role::Writer,
        "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/UpdateBranch" => Role::Writer,
        "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/ResolveBranchEntry" => Role::Writer,

        // Operation receipts are scoped to the caller's own claims, keyed
        // off the same principal every mutation above authenticates
        // against, so this classifies alongside them rather than with the
        // plain reads.
        "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/GetOperation" => Role::Writer,

        _ => return None,
    })
}

// ---------------------------------------------------------------------------
// TokenDigest
// ---------------------------------------------------------------------------

/// An API key held as a fixed-width digest rather than as its own bytes.
///
/// Two reasons this is not a `String`. `subtle`'s slice comparison documents
/// that it short-circuits when the lengths differ, so comparing raw tokens
/// answers faster for a wrong-length guess and leaks the length of the real
/// key. Hashing both sides to 32 bytes first makes every comparison examine
/// the same number of bytes. It also means the registry stops holding
/// plaintext secrets once startup is done, so a stray `Debug` of the server
/// state cannot print them.
///
/// This is a lookup key, not a password store: the input is a
/// high-entropy random token, so a plain digest is appropriate and a
/// deliberately slow KDF would only add per-request latency.
#[derive(Clone)]
pub struct TokenDigest([u8; 32]);

impl TokenDigest {
    /// Digest a presented or configured token.
    #[must_use]
    pub fn of(token: &str) -> Self {
        use sha2::{Digest as _, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(token.as_bytes());
        Self(hasher.finalize().into())
    }

    /// Constant-time equality. Deliberately the only way to compare two
    /// digests: no `PartialEq` is derived, so `==` cannot silently
    /// reintroduce a short-circuiting comparison on an authentication path.
    #[must_use]
    pub fn matches(&self, other: &Self) -> bool {
        self.0.ct_eq(&other.0).into()
    }
}

impl std::fmt::Debug for TokenDigest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TokenDigest(redacted)")
    }
}

// ---------------------------------------------------------------------------
// Token registry: TOML config
// ---------------------------------------------------------------------------

/// Per-principal config block inside the TOML token registry file.
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrincipalConfig {
    pub role: Role,
    /// `user` (the default) or `agent`. See [`PrincipalKind`]. Absent means
    /// `user`, so adding this field changes no existing registry's behavior.
    #[serde(default)]
    pub kind: PrincipalKind,
    /// API keys written inline. Convenient for tests and for a dev stack;
    /// for anything real prefer `token_files`, which keeps the secret out of
    /// this file.
    #[serde(default)]
    pub tokens: Vec<String>,
    /// Paths to files that each hold exactly one API key, so the secret lives
    /// in a mounted file rather than inline here or in an environment
    /// variable. Environment variables in particular are readable by anyone
    /// who can run `docker inspect`, and they are what an orchestrator prints
    /// back when asked to describe the service.
    ///
    /// A relative path resolves against the directory holding the registry
    /// file, not the server's working directory, so the registry stays valid
    /// no matter where the process is started from.
    ///
    /// Trailing newlines are stripped, because `echo secret > file` and every
    /// text editor add one and none of them mean it as part of the key.
    #[serde(default)]
    pub token_files: Vec<PathBuf>,
    /// Optional write allow-list, e.g. `namespaces = ["orders", "shop-*"]`.
    /// Absent or empty means unrestricted, so adding this field changes no
    /// existing registry's behavior.
    #[serde(default)]
    pub namespaces: Vec<String>,
    /// Optional ownership binding, e.g. `parent = "acme"`. Absent means the
    /// principal is unscoped, so adding this field changes no existing
    /// registry's behavior.
    ///
    /// Named `parent` per ADR#6310044131 rule 1: it is the principal's
    /// position in the ownership hierarchy, and a principal's parent is an
    /// owner rather than another principal, so the bare name is correct.
    #[serde(default)]
    pub parent: Option<String>,
}

/// Top-level structure of the token registry TOML file.
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegistryFile {
    pub principals: HashMap<String, PrincipalConfig>,
}

/// Validate that a principal name matches the same safe charset that
/// `sanitize_author_field` in service.rs enforces: no control characters, at
/// most 256 chars. We impose the same rule here because principal names appear
/// in logs, traces, and (in Phase 2) git mirror commits.
fn is_valid_principal_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 256 && !name.chars().any(char::is_control)
}

/// Read a single API key from a static secret file.
///
/// Only trailing CR/LF is stripped. Interior and leading whitespace is kept,
/// because trimming it would silently authenticate a key the operator did not
/// write, and a secret is the one value that must never be "helpfully"
/// rewritten.
fn read_token_file(path: &Path) -> Result<String, String> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| format!("reading token file {}: {e}", path.display()))?;
    let token = raw.trim_end_matches(['\n', '\r']).to_owned();
    if token.is_empty() {
        return Err(format!("token file {} is empty", path.display()));
    }
    warn_if_world_readable(path);
    Ok(token)
}

/// A secret file readable by every account on the host is almost always a
/// mistake, but it is a routine one inside a container image, so this warns
/// rather than refusing to start.
#[cfg(unix)]
fn warn_if_world_readable(path: &Path) {
    use std::os::unix::fs::PermissionsExt as _;
    let Ok(meta) = std::fs::metadata(path) else {
        return;
    };
    let mode = meta.permissions().mode();
    if mode & 0o077 != 0 {
        tracing::warn!(
            path = %path.display(),
            mode = format!("{:o}", mode & 0o777),
            "token file is readable by group or others; tighten it to 0600",
        );
    }
}

#[cfg(not(unix))]
fn warn_if_world_readable(_path: &Path) {}

/// Validated, lookup-ready token registry. Loaded once at startup.
#[derive(Debug, Clone)]
pub struct TokenRegistry {
    // Maps raw token bytes -> Principal. Arc so Clone is cheap.
    entries: Arc<Vec<(TokenDigest, Principal)>>,
    /// When true, every request is treated as an anonymous Admin. Used when
    /// `--insecure-allow-anonymous` is active and no registry file is given.
    pub anonymous_admin: bool,
}

impl TokenRegistry {
    /// Build a registry from a parsed TOML file. Validates:
    /// - Principal names pass the safe-charset check.
    /// - No token string appears under more than one principal.
    /// - Every namespace pattern is one a namespace could actually match.
    ///
    /// # Errors
    ///
    /// Returns an error string when validation fails.
    pub fn from_file_contents(file: RegistryFile) -> Result<Self, String> {
        Self::from_file_contents_in(file, Path::new(""))
    }

    /// Build a registry, resolving relative `token_files` against `base_dir`.
    ///
    /// Same validation as [`Self::from_file_contents`], plus: every declared
    /// token file must exist, be readable, and be non-empty. A missing secret
    /// file fails startup rather than quietly producing a principal nobody
    /// can authenticate as.
    ///
    /// # Errors
    ///
    /// Returns an error string when validation fails or a token file cannot
    /// be read.
    pub fn from_file_contents_in(file: RegistryFile, base_dir: &Path) -> Result<Self, String> {
        let mut entries: Vec<(TokenDigest, Principal)> = Vec::new();
        // Keyed on the plaintext token, which only exists during load. The
        // value records where the token came from so a duplicate can name
        // both sources instead of just saying "somewhere".
        let mut seen_tokens: HashMap<String, String> = HashMap::new();

        for (name, cfg) in file.principals {
            if !is_valid_principal_name(&name) {
                return Err(format!(
                    "principal name {name:?} contains control characters or is empty/too long",
                ));
            }
            // A pattern that can never match is almost always a typo, and a
            // typo here silently widens or narrows a grant. Fail startup.
            let namespaces = crate::scope::NamespaceScope::parse(&cfg.namespaces)
                .map_err(|e| format!("principal {name:?}: {e}"))?;
            // Same reasoning as the pattern check above: a malformed owner is
            // a typo, and a typo in a tenancy boundary is the one config
            // error that must never start.
            let parent = cfg
                .parent
                .as_deref()
                .map(trogon_atlas_core::OwnerId::parse)
                .transpose()
                .map_err(|e| format!("principal {name:?}: invalid parent: {e}"))?;
            // Moving a namespace between owners is the only operation that
            // crosses a tenancy boundary, which is why it is restricted to
            // `admin`. Binding an admin to a `parent` would make that boundary
            // the very thing the principal sits inside of, so the config that
            // could do that is refused rather than trusted to the handler.
            if cfg.role == Role::Admin && parent.is_some() {
                return Err(format!(
                    "principal {name:?} is role=admin and has a parent; admin must not be bound to an owner",
                ));
            }
            // An agent with no parent can never satisfy `authorizes_owner`:
            // `PrincipalKind::Agent` only matches through `parent`, so a
            // parentless agent is permanently locked out of every owned
            // branch. That is a dead configuration, not a narrower grant, so
            // it fails startup the same way an inherently-unmatchable
            // namespace pattern does.
            if cfg.kind == PrincipalKind::Agent && parent.is_none() {
                return Err(format!(
                    "principal {name:?} is kind=agent and has no parent; an agent must be delegated by an owner",
                ));
            }
            // Inline tokens and file-sourced tokens are the same thing once
            // read, and both must be deduplicated against each other, so they
            // are gathered into one list before validation.
            let mut tokens: Vec<(String, String)> = cfg
                .tokens
                .into_iter()
                .map(|t| (t, format!("principal {name:?}")))
                .collect();
            for relative in cfg.token_files {
                let path = base_dir.join(&relative);
                let token =
                    read_token_file(&path).map_err(|e| format!("principal {name:?}: {e}"))?;
                tokens.push((token, format!("token file {}", path.display())));
            }

            if tokens.is_empty() {
                return Err(format!(
                    "principal {name:?} declares no tokens; set `tokens` or `token_files`",
                ));
            }

            for (token, source) in tokens {
                // Empty tokens collide with BearerAuth's missing-header path
                // (absent Authorization was historically treated as `""`).
                if token.is_empty() {
                    return Err(format!(
                        "principal {name:?} has an empty token; empty tokens are forbidden",
                    ));
                }
                if let Some(existing) = seen_tokens.get(&token) {
                    return Err(format!(
                        "the same token is declared by {existing} and by {source}; each token must be unique",
                    ));
                }
                seen_tokens.insert(token.clone(), source);
                entries.push((
                    TokenDigest::of(&token),
                    Principal {
                        name: Arc::from(name.as_str()),
                        role: cfg.role,
                        kind: cfg.kind,
                        is_anonymous: false,
                        namespaces: namespaces.clone(),
                        parent: parent.clone(),
                    },
                ));
            }
        }

        Ok(Self {
            entries: Arc::new(entries),
            anonymous_admin: false,
        })
    }

    /// Load and validate a registry from the given file path.
    ///
    /// # Errors
    ///
    /// Returns an error string when the file cannot be read, parsed, or fails validation.
    pub fn load(path: &Path) -> Result<Self, String> {
        let contents = std::fs::read_to_string(path)
            .map_err(|e| format!("reading token registry {}: {e}", path.display()))?;
        let file: RegistryFile = toml::from_str(&contents)
            .map_err(|e| format!("parsing token registry {}: {e}", path.display()))?;
        // Against the registry's own directory, so moving the pair of files
        // together keeps them working and the server's cwd never matters.
        let base = path.parent().unwrap_or_else(|| Path::new(""));
        Self::from_file_contents_in(file, base)
    }

    /// The principal-to-owner bindings this registry declares, deduplicated
    /// across the tokens that share a principal.
    ///
    /// The token file is the authority for who a principal is and which owner
    /// it belongs to. An external authorizer has to be told that mapping,
    /// because it cannot read the file, and this is the only place the
    /// mapping exists.
    #[must_use]
    pub fn memberships(&self) -> Vec<(Arc<str>, trogon_atlas_core::OwnerId)> {
        let mut seen: HashSet<(String, String)> = HashSet::new();
        let mut out = Vec::new();
        for (_, principal) in self.entries.iter() {
            let Some(parent) = principal.parent.as_ref() else {
                continue;
            };
            if seen.insert((principal.name.to_string(), parent.as_str().to_string())) {
                out.push((principal.name.clone(), parent.clone()));
            }
        }
        out
    }

    /// A registry where every request is authenticated as an anonymous Admin.
    /// Used when `--insecure-allow-anonymous` is active without a tokens file.
    #[must_use]
    pub fn anonymous() -> Self {
        Self {
            entries: Arc::new(Vec::new()),
            anonymous_admin: true,
        }
    }

    /// A registry containing exactly one synthetic principal from the legacy
    /// `--auth-token` / `TROGON_ATLAS_AUTH_TOKEN` argument.
    #[must_use]
    pub fn from_legacy_token(token: &str) -> Self {
        let entries = vec![(
            TokenDigest::of(token),
            Principal {
                name: Arc::from("legacy-admin"),
                role: Role::Admin,
                kind: PrincipalKind::User,
                is_anonymous: false,
                namespaces: crate::scope::NamespaceScope::unrestricted(),
                parent: None,
            },
        )];
        Self {
            entries: Arc::new(entries),
            anonymous_admin: false,
        }
    }

    /// True when `other` authenticates exactly the same tokens as the same
    /// principals. Used by the reloader to tell an edited registry from a
    /// touched-but-identical one, so a no-op write does not churn SpiceDB or
    /// print a misleading "reloaded" line.
    ///
    /// Comparison is by content, not by file bytes: a reordered or
    /// reformatted registry that grants the same access is not a change.
    /// Token equality goes through [`TokenDigest::matches`] because that is
    /// the only comparison this type offers. Entry-for-entry search is
    /// quadratic, which is fine at registry sizes and correct as a multiset
    /// test only because `from_file_contents_in` has already rejected
    /// duplicate tokens.
    #[must_use]
    pub fn grants_same_as(&self, other: &Self) -> bool {
        self.anonymous_admin == other.anonymous_admin
            && self.entries.len() == other.entries.len()
            && self.entries.iter().all(|(digest, principal)| {
                other.entries.iter().any(|(candidate, other_principal)| {
                    candidate.matches(digest) && other_principal == principal
                })
            })
    }

    /// Look up a bearer token using constant-time comparison per candidate.
    /// Returns the resolved `Principal` if found, or `None`.
    #[must_use]
    pub fn lookup(&self, provided: &str) -> Option<Principal> {
        // Empty provided tokens must never authenticate. An empty registered
        // token would otherwise match missing Authorization (`unwrap_or("")`).
        if provided.is_empty() {
            return None;
        }
        // Always iterate every entry so we do not short-circuit on position,
        // which would leak which token matched via timing.
        let provided = TokenDigest::of(provided);
        let mut found: Option<Principal> = None;
        for (candidate, principal) in self.entries.as_ref() {
            if candidate.matches(&provided) {
                found = Some(principal.clone());
            }
        }
        found
    }
}

// ---------------------------------------------------------------------------
// BearerAuth interceptor
// ---------------------------------------------------------------------------

/// Tonic interceptor that resolves the bearer token to a `Principal` and
/// inserts it into request extensions. Unknown tokens result in
/// UNAUTHENTICATED.
///
/// Authorization (role >= `required_role`) is enforced by `AuthzLayer` further
/// down the tower stack where the URI path is accessible.
/// The registry currently in force, swappable while the server is serving.
///
/// A [`TokenRegistry`] is an immutable snapshot on purpose: the authentication
/// path must never observe a half-applied edit. Reloading therefore builds a
/// whole new snapshot and swaps the pointer, so an in-flight request keeps
/// reading the registry it started with and the next one reads the new one.
///
/// The lock is held only long enough to clone an `Arc`, never across a
/// lookup, so a reload cannot stall authentication.
#[derive(Clone)]
pub struct LiveRegistry(Arc<parking_lot::RwLock<Arc<TokenRegistry>>>);

impl LiveRegistry {
    #[must_use]
    pub fn new(registry: Arc<TokenRegistry>) -> Self {
        Self(Arc::new(parking_lot::RwLock::new(registry)))
    }

    /// The snapshot in force right now.
    #[must_use]
    pub fn current(&self) -> Arc<TokenRegistry> {
        self.0.read().clone()
    }

    /// Put a new snapshot in force. Takes effect on the next request; nothing
    /// already in flight is affected.
    pub fn install(&self, registry: Arc<TokenRegistry>) {
        *self.0.write() = registry;
    }
}

impl std::fmt::Debug for LiveRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("LiveRegistry")
            .field(&self.current())
            .finish()
    }
}

impl From<Arc<TokenRegistry>> for LiveRegistry {
    fn from(registry: Arc<TokenRegistry>) -> Self {
        Self::new(registry)
    }
}

impl From<TokenRegistry> for LiveRegistry {
    fn from(registry: TokenRegistry) -> Self {
        Self::new(Arc::new(registry))
    }
}

#[derive(Clone)]
pub struct BearerAuth {
    registry: LiveRegistry,
}

impl BearerAuth {
    pub fn new(registry: impl Into<LiveRegistry>) -> Self {
        Self {
            registry: registry.into(),
        }
    }

    /// The handle this interceptor authenticates against, for a reloader to
    /// swap snapshots into.
    #[must_use]
    pub fn live(&self) -> LiveRegistry {
        self.registry.clone()
    }
}

impl tonic::service::Interceptor for BearerAuth {
    fn call(&mut self, mut req: Request<()>) -> Result<Request<()>, Status> {
        let registry = self.registry.current();
        if registry.anonymous_admin {
            req.extensions_mut().insert(Principal {
                name: Arc::from("anonymous"),
                role: Role::Admin,
                kind: PrincipalKind::User,
                is_anonymous: true,
                namespaces: crate::scope::NamespaceScope::unrestricted(),
                parent: None,
            });
            return Ok(req);
        }

        let Some(provided) = req
            .metadata()
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
        else {
            return Err(Status::unauthenticated("invalid or missing bearer token"));
        };

        match registry.lookup(provided) {
            Some(principal) => {
                req.extensions_mut().insert(principal);
                Ok(req)
            }
            None => Err(Status::unauthenticated("invalid or missing bearer token")),
        }
    }
}

// ---------------------------------------------------------------------------
// AuthzLayer: tower middleware that enforces role against the URI path
// ---------------------------------------------------------------------------

/// Tower layer that wraps a service and enforces `required_role` against the
/// HTTP request path. Must be placed after the `BearerAuth` interceptor so
/// `Principal` is already in extensions.
#[derive(Clone)]
pub struct AuthzLayer;

impl<S> Layer<S> for AuthzLayer {
    type Service = AuthzService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        AuthzService { inner }
    }
}

#[derive(Clone)]
pub struct AuthzService<S> {
    inner: S,
}

impl<S: tonic::server::NamedService> tonic::server::NamedService for AuthzService<S> {
    const NAME: &'static str = S::NAME;
}

impl<S, ReqBody, ResBody> Service<http::Request<ReqBody>> for AuthzService<S>
where
    S: Service<http::Request<ReqBody>, Response = http::Response<ResBody>> + Clone + Send + 'static,
    S::Future: Send + 'static,
    S::Error: Send + 'static,
    ReqBody: Send + 'static,
    ResBody: Default + http_body::Body + Send + 'static,
{
    type Response = http::Response<ResBody>;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: http::Request<ReqBody>) -> Self::Future {
        let path = req.uri().path();
        let required = required_role(path);

        let caller_role: Option<Role> = req.extensions().get::<Principal>().map(|p| p.role);

        let authorized = match (caller_role, required) {
            // A principal with a role >= the required role is permitted.
            (Some(caller), Some(req_role)) => caller >= req_role,
            // Path not in the map, or no principal in extensions: deny.
            // No-principal case: BearerAuth should have rejected first, but
            // we deny here as defense-in-depth.
            (_, None) | (None, _) => false,
        };

        if authorized {
            let clone = self.inner.clone();
            let mut inner = std::mem::replace(&mut self.inner, clone);
            Box::pin(async move { inner.call(req).await })
        } else {
            let status = if required.is_none() {
                Status::permission_denied("method not mapped in authorization table")
            } else {
                Status::permission_denied("caller role insufficient for this RPC")
            };
            // Encode PERMISSION_DENIED as gRPC trailers-only response.
            // tonic normally places grpc-status in HTTP/2 trailers; for
            // tower-level rejections we emit it as headers in a trailers-only
            // response (grpc-status in headers is valid per the gRPC-over-HTTP/2
            // spec when there is no message body).
            let mut resp = http::Response::new(ResBody::default());
            let headers = resp.headers_mut();
            headers.insert(
                "content-type",
                http::HeaderValue::from_static("application/grpc"),
            );
            // gRPC status codes are small integers (0-16); from_static is not
            // applicable for a runtime value, so we use a static fallback for
            // PERMISSION_DENIED (code 7) in the rare case from_str errors.
            let code_str = (status.code() as i32).to_string();
            if let Ok(v) = http::HeaderValue::from_str(&code_str) {
                headers.insert("grpc-status", v);
            } else {
                headers.insert("grpc-status", http::HeaderValue::from_static("7"));
            }
            if !status.message().is_empty() {
                if let Ok(v) = http::HeaderValue::from_str(status.message()) {
                    headers.insert("grpc-message", v);
                }
            }
            Box::pin(std::future::ready(Ok(resp)))
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    // ---- Role ordering -------------------------------------------------------

    #[test]
    fn role_ordering_reader_lt_writer_lt_admin() {
        assert!(Role::Reader < Role::Writer);
        assert!(Role::Writer < Role::Admin);
        assert!(Role::Reader < Role::Admin);
    }

    #[test]
    fn role_equality() {
        assert_eq!(Role::Reader, Role::Reader);
        assert_eq!(Role::Writer, Role::Writer);
        assert_eq!(Role::Admin, Role::Admin);
    }

    // ---- Registry parsing: happy path ----------------------------------------

    fn parse_toml(raw: &str) -> TokenRegistry {
        let file: RegistryFile = toml::from_str(raw).expect("valid TOML");
        TokenRegistry::from_file_contents(file).expect("valid registry")
    }

    #[test]
    fn registry_happy_path() {
        let r = parse_toml(
            r#"
[principals.studio-gateway]
role = "reader"
tokens = ["aaaa"]

[principals.mcp]
role = "writer"
tokens = ["bbbb", "cccc"]

[principals.alex-cli]
role = "admin"
tokens = ["dddd"]
"#,
        );
        assert!(r.lookup("aaaa").is_some());
        assert_eq!(r.lookup("aaaa").unwrap().role, Role::Reader);
        assert_eq!(r.lookup("bbbb").unwrap().role, Role::Writer);
        assert_eq!(r.lookup("cccc").unwrap().role, Role::Writer);
        assert_eq!(r.lookup("dddd").unwrap().role, Role::Admin);
        assert!(r.lookup("xxxx").is_none());
    }

    // ---- Registry parsing: bad role ------------------------------------------

    #[test]
    fn registry_bad_role_is_toml_error() {
        let raw = r#"
[principals.bad]
role = "superuser"
tokens = ["aaaa"]
"#;
        let result: Result<RegistryFile, _> = toml::from_str(raw);
        assert!(result.is_err(), "unknown role variant must fail TOML parse");
    }

    // ---- Registry parsing: duplicate token -----------------------------------

    #[test]
    fn registry_duplicate_token_across_principals_is_error() {
        let file: RegistryFile = toml::from_str(
            r#"
[principals.alice]
role = "reader"
tokens = ["shared"]

[principals.bob]
role = "writer"
tokens = ["shared"]
"#,
        )
        .unwrap();
        let result = TokenRegistry::from_file_contents(file);
        assert!(result.is_err(), "duplicate token must be rejected");
        let msg = result.unwrap_err();
        // Error must name both principals so the operator knows where the conflict is.
        assert!(
            msg.contains("alice") || msg.contains("bob"),
            "error should mention principal names; got: {msg}"
        );
    }

    // ---- Registry parsing: unknown TOML field --------------------------------

    #[test]
    fn registry_unknown_field_is_error() {
        let raw = r#"
[principals.alice]
role = "reader"
tokens = ["aaaa"]
extra_field = "oops"
"#;
        let result: Result<RegistryFile, _> = toml::from_str(raw);
        assert!(
            result.is_err(),
            "unknown field must fail with deny_unknown_fields"
        );
    }

    // ---- Constant-time lookup ------------------------------------------------

    #[test]
    fn lookup_correct_token() {
        let r = parse_toml(
            r#"
[principals.p]
role = "admin"
tokens = ["correct"]
"#,
        );
        let p = r.lookup("correct").expect("should resolve");
        assert_eq!(p.role, Role::Admin);
        assert_eq!(p.name.as_ref(), "p");
    }

    #[test]
    fn lookup_wrong_token_returns_none() {
        let r = parse_toml(
            r#"
[principals.p]
role = "admin"
tokens = ["correct"]
"#,
        );
        assert!(r.lookup("wrong").is_none());
        assert!(r.lookup("").is_none());
        assert!(r.lookup("correc").is_none());
        assert!(r.lookup("correctx").is_none());
    }

    #[test]
    fn lookup_iterates_all_candidates_for_constant_time() {
        // Add multiple entries to verify all are checked.
        let r = parse_toml(
            r#"
[principals.a]
role = "reader"
tokens = ["aaa"]

[principals.b]
role = "writer"
tokens = ["bbb"]

[principals.c]
role = "admin"
tokens = ["ccc"]
"#,
        );
        assert_eq!(r.lookup("aaa").unwrap().role, Role::Reader);
        assert_eq!(r.lookup("bbb").unwrap().role, Role::Writer);
        assert_eq!(r.lookup("ccc").unwrap().role, Role::Admin);
        assert!(r.lookup("ddd").is_none());
    }

    // ---- Invalid principal name ----------------------------------------------

    #[test]
    fn registry_control_char_in_principal_name_is_error() {
        // Build a RegistryFile directly to inject a control character in the
        // principal name without fighting TOML string validation.
        let mut principals = HashMap::new();
        principals.insert(
            "bad\tname".to_owned(), // tab is a control character
            PrincipalConfig {
                role: Role::Reader,
                kind: PrincipalKind::User,
                tokens: vec!["aaaa".to_owned()],
                token_files: Vec::new(),
                namespaces: Vec::new(),
                parent: None,
            },
        );
        let file = RegistryFile { principals };
        let result = TokenRegistry::from_file_contents(file);
        assert!(
            result.is_err(),
            "control char in principal name must be rejected"
        );
    }

    // ---- Namespace scope -----------------------------------------------------

    #[test]
    fn registry_without_namespaces_yields_an_unrestricted_principal() {
        let toml = r#"
[principals.legacy]
role = "writer"
tokens = ["legacy-token"]
"#;
        let file: RegistryFile = toml::from_str(toml).expect("valid TOML");
        let registry = TokenRegistry::from_file_contents(file).expect("valid registry");
        let principal = registry.lookup("legacy-token").expect("token resolves");
        assert!(
            principal.namespaces.is_unrestricted(),
            "a registry written before the field existed must keep writing everywhere"
        );
    }

    #[test]
    fn registry_namespaces_reach_the_principal() {
        let toml = r#"
[principals.orders-team]
role = "writer"
tokens = ["orders-token"]
namespaces = ["orders", "shop-*"]
"#;
        let file: RegistryFile = toml::from_str(toml).expect("valid TOML");
        let registry = TokenRegistry::from_file_contents(file).expect("valid registry");
        let principal = registry.lookup("orders-token").expect("token resolves");
        assert!(!principal.namespaces.is_unrestricted());
        assert!(principal.namespaces.admits("orders"));
        assert!(principal.namespaces.admits("shop-checkout"));
        assert!(!principal.namespaces.admits("billing"));
    }

    // A pattern that can never match a namespace is a typo, and a typo in an
    // allow-list silently changes who may write what. Fail startup instead.
    #[test]
    fn registry_rejects_a_namespace_pattern_no_namespace_could_match() {
        let toml = r#"
[principals.typo]
role = "writer"
tokens = ["typo-token"]
namespaces = ["orders/*"]
"#;
        let file: RegistryFile = toml::from_str(toml).expect("valid TOML");
        let err = TokenRegistry::from_file_contents(file)
            .expect_err("an unmatchable pattern must abort startup");
        assert!(
            err.contains("typo"),
            "the error must name the principal: {err}"
        );
    }

    // Empty registered tokens collide with BearerAuth's missing-header path
    // (`strip_prefix(...).unwrap_or("")`), so an empty token would authenticate
    // every unauthenticated request as that principal.
    #[test]
    fn registry_empty_token_must_not_authenticate_missing_authorization() {
        let mut principals = HashMap::new();
        principals.insert(
            "accident".to_owned(),
            PrincipalConfig {
                role: Role::Admin,
                kind: PrincipalKind::User,
                tokens: vec![String::new()],
                token_files: Vec::new(),
                namespaces: Vec::new(),
                parent: None,
            },
        );
        let file = RegistryFile { principals };
        // Empty tokens should be rejected at load; if load succeeds, lookup must still deny.
        match TokenRegistry::from_file_contents(file) {
            Err(_) => {}
            Ok(registry) => {
                assert!(
                    registry.lookup("").is_none(),
                    "missing/empty Authorization must not match an empty registered token \
                     (BearerAuth uses unwrap_or(\"\") for absent headers)"
                );
            }
        }
    }

    // ---- Legacy token constructor ---------------------------------------------

    #[test]
    fn legacy_token_creates_admin_principal() {
        let r = TokenRegistry::from_legacy_token("my-legacy-token");
        let p = r
            .lookup("my-legacy-token")
            .expect("should find legacy token");
        assert_eq!(p.role, Role::Admin);
        assert_eq!(p.name.as_ref(), "legacy-admin");
    }

    // ---- Anonymous mode ------------------------------------------------------

    #[test]
    fn anonymous_registry_lookup_returns_none() {
        // lookup() is irrelevant in anonymous mode; the interceptor short-
        // circuits. But validate it doesn't crash.
        let r = TokenRegistry::anonymous();
        assert!(r.lookup("anything").is_none());
        assert!(r.anonymous_admin);
    }

    // ---- required_role: spot checks ------------------------------------------

    #[test]
    fn required_role_read_rpcs() {
        let read_rpcs = [
            "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/GetServerInfo",
            "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/ListNamespaces",
            "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/WhoAmI",
            "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/CompileTypeLibrary",
            "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/ResolveType",
            "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/GetTypeLibraryDescriptorSet",
            "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/GetEntity",
            "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/BatchGetEntities",
            "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/ListEntities",
            "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/SearchEntities",
            "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/ListVersions",
            "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/GetLatestVersion",
            "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/GetSupersessionChain",
            "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/GetIncomingReferences",
            "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/GetOutgoingReferences",
            "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/GetImpact",
            "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/GetSliceProjection",
            "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/GetStoryboardProjection",
            "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/GetEventModelProjection",
            "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/ExtractSubgraph",
            "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/DiffEntities",
            "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/ListChanges",
            "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/StreamChanges",
            "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/ListChangesets",
            "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/GetChangeset",
            "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/GetEntityHistory",
            "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/ValidateEventModel",
            "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/ListValidationRules",
            "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/ValidateProject",
            "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/ListEntityKinds",
            "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/ListEntitiesByDomain",
            "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/ListBranches",
            "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/DiffBranch",
        ];
        for path in read_rpcs {
            assert_eq!(
                required_role(path),
                Some(Role::Reader),
                "expected Reader for {path}"
            );
        }
    }

    #[test]
    fn required_role_writer_rpcs() {
        let writer_rpcs = [
            "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/PutEntity",
            "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/DeleteEntity",
            "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/BatchMutate",
            "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/RevertChangeset",
            "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/InferDataFlow",
            "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/CheckInformationCompleteness",
        ];
        for path in writer_rpcs {
            assert_eq!(
                required_role(path),
                Some(Role::Writer),
                "expected Writer for {path}"
            );
        }
    }

    #[test]
    fn required_role_admin_rpcs() {
        assert_eq!(
            required_role("/trogonatlas.api.eventmodel.v1alpha1.EventModelService/DeleteByQuery"),
            Some(Role::Admin)
        );
    }

    #[test]
    fn required_role_unknown_path_returns_none() {
        assert_eq!(
            required_role("/trogonatlas.api.eventmodel.v1alpha1.EventModelService/NoSuchRPC"),
            None
        );
        assert_eq!(required_role("/other.Service/Method"), None);
        assert_eq!(required_role(""), None);
    }

    // ---- Descriptor coverage -------------------------------------------------
    //
    // This test asserts that every RPC in the EventModelService has a
    // required_role entry. It parses `trogon_atlas_proto::FILE_DESCRIPTOR_SET`
    // using `prost_reflect` to enumerate methods, then checks each one.
    //
    // If `prost_reflect` cannot decode the descriptor (should not happen in
    // practice), the test is skipped with a clear message rather than failing.
    //
    // The canonical list of RPCs lives in:
    //   proto/trogonatlas/api/eventmodel/v1alpha1/service.proto
    // Keep required_role() in sync with that file whenever a new RPC is added.
    #[test]
    fn all_service_methods_are_mapped() {
        use prost_reflect::{DescriptorPool, ServiceDescriptor};

        let pool = DescriptorPool::decode(trogon_atlas_proto::FILE_DESCRIPTOR_SET)
            .expect("FILE_DESCRIPTOR_SET must be a valid FileDescriptorSet");

        let svc: ServiceDescriptor = pool
            .get_service_by_name("trogonatlas.api.eventmodel.v1alpha1.EventModelService")
            .expect("EventModelService must exist in the descriptor pool");

        let mut unmapped: Vec<String> = Vec::new();
        for method in svc.methods() {
            let path = format!(
                "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/{}",
                method.name()
            );
            if required_role(&path).is_none() {
                unmapped.push(path);
            }
        }

        assert!(
            unmapped.is_empty(),
            "The following RPCs have no required_role entry and will be \
             PERMISSION_DENIED for all callers. Add them to required_role() \
             in src/auth.rs:\n{}",
            unmapped.join("\n")
        );
    }

    // ---- API keys from a static secret file ----------------------------------

    /// Each test gets its own directory so a leftover file from a previous
    /// run cannot make a later one pass.
    fn secret_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "trogon-atlas-token-file-{tag}-{}",
            uuid::Uuid::now_v7()
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn write_secret(dir: &std::path::Path, name: &str, contents: &str) -> std::path::PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, contents).expect("write secret");
        path
    }

    /// The headline behaviour: the key lives in its own file, and the
    /// principal's role, scope, and owner still come from the registry.
    #[test]
    fn a_token_file_authenticates_and_carries_the_principals_grants() {
        let dir = secret_dir("happy");
        let secret = write_secret(&dir, "api.key", "s3cret-from-a-file");
        let raw = format!(
            r#"
[principals.ci]
role = "writer"
token_files = ["{}"]
namespaces = ["orders"]
parent = "acme"
"#,
            secret.display()
        );
        let file: RegistryFile = toml::from_str(&raw).expect("valid TOML");
        let r = TokenRegistry::from_file_contents(file).expect("valid registry");

        let p = r
            .lookup("s3cret-from-a-file")
            .expect("the file's contents must authenticate");
        assert_eq!(&*p.name, "ci");
        assert_eq!(p.role, Role::Writer);
        assert_eq!(
            p.parent.as_ref().map(trogon_atlas_core::OwnerId::as_str),
            Some("acme")
        );
        assert!(!p.is_anonymous);
        assert!(r.lookup("s3cret-from-a-file-x").is_none());
        std::fs::remove_dir_all(&dir).ok();
    }

    /// `echo secret > file` appends a newline and no operator means it as part
    /// of the key, so the trailing newline must not be significant.
    #[test]
    fn a_trailing_newline_in_the_secret_file_is_not_part_of_the_key() {
        let dir = secret_dir("newline");
        let secret = write_secret(&dir, "api.key", "trimmed-key\n");
        let raw = format!(
            "[principals.ci]\nrole = \"reader\"\ntoken_files = [\"{}\"]\n",
            secret.display()
        );
        let file: RegistryFile = toml::from_str(&raw).expect("valid TOML");
        let r = TokenRegistry::from_file_contents(file).expect("valid registry");

        assert!(r.lookup("trimmed-key").is_some());
        assert!(
            r.lookup("trimmed-key\n").is_none(),
            "the newline must not be accepted as part of the key either",
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A relative path must follow the registry file, not the process's cwd,
    /// or the same config would mean different things in different stacks.
    #[test]
    fn a_relative_token_file_resolves_against_the_registry_directory() {
        let dir = secret_dir("relative");
        write_secret(&dir, "api.key", "relative-key");
        let registry = write_secret(
            &dir,
            "tokens.toml",
            "[principals.ci]\nrole = \"admin\"\ntoken_files = [\"api.key\"]\n",
        );

        let r = TokenRegistry::load(&registry).expect("registry loads");
        assert!(
            r.lookup("relative-key").is_some(),
            "a bare filename must resolve next to the registry that names it",
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A typo'd path must stop the server. Starting with a principal nobody
    /// can authenticate as is the failure mode where an operator believes a
    /// key is live when it is not.
    #[test]
    fn a_missing_token_file_fails_startup() {
        let dir = secret_dir("missing");
        let raw = format!(
            "[principals.ci]\nrole = \"reader\"\ntoken_files = [\"{}\"]\n",
            dir.join("absent.key").display()
        );
        let file: RegistryFile = toml::from_str(&raw).expect("valid TOML");
        let err = TokenRegistry::from_file_contents(file)
            .expect_err("a missing secret file must not start");
        assert!(
            err.contains("absent.key"),
            "the error must name the path the operator has to fix, got: {err}",
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// An empty file would otherwise register an empty token, which is the
    /// exact value `BearerAuth` uses for a missing Authorization header.
    #[test]
    fn an_empty_token_file_fails_startup() {
        let dir = secret_dir("empty");
        let secret = write_secret(&dir, "api.key", "\n");
        let raw = format!(
            "[principals.ci]\nrole = \"reader\"\ntoken_files = [\"{}\"]\n",
            secret.display()
        );
        let file: RegistryFile = toml::from_str(&raw).expect("valid TOML");
        let err = TokenRegistry::from_file_contents(file)
            .expect_err("an empty secret file must not start");
        assert!(err.contains("is empty"), "got: {err}");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Uniqueness has to hold across both sources, otherwise one key could
    /// resolve to two principals depending on iteration order.
    #[test]
    fn a_token_duplicated_between_a_file_and_an_inline_entry_is_refused() {
        let dir = secret_dir("dup");
        let secret = write_secret(&dir, "api.key", "shared-key");
        let raw = format!(
            r#"
[principals.ci]
role = "writer"
token_files = ["{}"]

[principals.studio]
role = "reader"
tokens = ["shared-key"]
"#,
            secret.display()
        );
        let file: RegistryFile = toml::from_str(&raw).expect("valid TOML");
        let err = TokenRegistry::from_file_contents(file)
            .expect_err("the same key under two principals must not start");
        assert!(
            err.contains("api.key") && err.contains("studio"),
            "the error must name both sources so the operator can find them, got: {err}",
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A principal with neither source is unreachable, which is a typo rather
    /// than an intent.
    #[test]
    fn a_principal_with_no_tokens_at_all_is_refused() {
        let file: RegistryFile =
            toml::from_str("[principals.ci]\nrole = \"admin\"\n").expect("valid TOML");
        let err = TokenRegistry::from_file_contents(file)
            .expect_err("a principal with no way to authenticate must not start");
        assert!(err.contains("declares no tokens"), "got: {err}");
    }

    /// The shipped example is the documentation for this file format, and
    /// `deny_unknown_fields` means a renamed or misspelled field there is a
    /// parse error rather than a silently ignored line. It is only parsed,
    /// not loaded: its `token_files` point at a secret mount that exists on a
    /// deployed host, not in the test environment.
    #[test]
    fn the_shipped_example_registry_still_parses() {
        let raw = include_str!("../examples/tokens.toml");
        let file: RegistryFile =
            toml::from_str(raw).expect("examples/tokens.toml must parse as a registry");
        let ci = file
            .principals
            .get("ci")
            .expect("the example must keep demonstrating token_files");
        assert_eq!(ci.token_files.len(), 1);
        assert!(
            ci.tokens.is_empty(),
            "the token_files example must not also inline a key, or it stops \
             showing what it is for",
        );
    }

    // ---- Digest comparison ---------------------------------------------------

    /// The reason tokens are digested: `subtle`'s slice comparison documents
    /// that it short-circuits on a length mismatch, so comparing raw tokens
    /// would answer a wrong-length guess faster and leak the real length.
    /// Every comparison now runs over 32 bytes regardless of input size.
    #[test]
    fn digests_are_fixed_width_regardless_of_token_length() {
        let short = TokenDigest::of("a");
        let long = TokenDigest::of(&"a".repeat(4096));
        assert_eq!(short.0.len(), long.0.len());
        assert!(!short.matches(&long));
        assert!(short.matches(&TokenDigest::of("a")));
    }

    /// The registry derives `Debug`, so anything it holds can reach a log line
    /// or a panic message.
    #[test]
    fn a_digest_never_prints_its_bytes() {
        let printed = format!("{:?}", TokenDigest::of("super-secret"));
        assert_eq!(printed, "TokenDigest(redacted)");

        let r = parse_toml("[principals.ci]\nrole = \"admin\"\ntokens = [\"super-secret\"]\n");
        let dumped = format!("{r:?}");
        assert!(
            !dumped.contains("super-secret"),
            "the registry must not print plaintext tokens, got: {dumped}",
        );
    }

    // ---- PrincipalKind ---------------------------------------------------

    #[test]
    fn kind_absent_in_toml_defaults_to_user() {
        let r = parse_toml("[principals.ci]\nrole = \"reader\"\ntokens = [\"aaaa\"]\n");
        assert_eq!(r.lookup("aaaa").unwrap().kind, PrincipalKind::User);
    }

    #[test]
    fn kind_agent_requires_a_parent() {
        let file: RegistryFile = toml::from_str(
            r#"
[principals.bot]
role = "writer"
kind = "agent"
tokens = ["aaaa"]
"#,
        )
        .unwrap();
        let err = TokenRegistry::from_file_contents(file).unwrap_err();
        assert!(
            err.contains("bot") && err.contains("parent"),
            "error should name the principal and the missing parent; got: {err}"
        );
    }

    #[test]
    fn kind_agent_with_a_parent_loads() {
        let r = parse_toml(
            r#"
[principals.bot]
role = "writer"
kind = "agent"
tokens = ["aaaa"]
parent = "alice"
"#,
        );
        let p = r.lookup("aaaa").unwrap();
        assert_eq!(p.kind, PrincipalKind::Agent);
        assert_eq!(
            p.parent.as_ref().map(trogon_atlas_core::OwnerId::as_str),
            Some("alice")
        );
    }

    /// A user acting as the owner itself: matched by name, not by `parent`.
    #[test]
    fn user_authorizes_the_owner_it_is_literally_named() {
        let alice = Principal {
            name: Arc::from("alice"),
            role: Role::Writer,
            kind: PrincipalKind::User,
            is_anonymous: false,
            namespaces: crate::scope::NamespaceScope::unrestricted(),
            parent: None,
        };
        assert!(alice.authorizes_owner(&trogon_atlas_core::OwnerId::parse("alice").unwrap()));
        assert!(!alice.authorizes_owner(&trogon_atlas_core::OwnerId::parse("bob").unwrap()));
    }

    /// An agent delegated by alice may act for alice, and only alice -- the
    /// negative case the delegation model exists to produce.
    #[test]
    fn agent_authorizes_only_the_owner_that_delegated_to_it() {
        let agent = Principal {
            name: Arc::from("some-bot"),
            role: Role::Writer,
            kind: PrincipalKind::Agent,
            is_anonymous: false,
            namespaces: crate::scope::NamespaceScope::unrestricted(),
            parent: Some(trogon_atlas_core::OwnerId::parse("alice").unwrap()),
        };
        assert!(agent.authorizes_owner(&trogon_atlas_core::OwnerId::parse("alice").unwrap()));
        assert!(!agent.authorizes_owner(&trogon_atlas_core::OwnerId::parse("bob").unwrap()));
    }

    /// An agent's own name carries no weight: only delegation does.
    #[test]
    fn agent_named_the_same_as_an_owner_still_needs_delegation() {
        let agent = Principal {
            name: Arc::from("bob"),
            role: Role::Writer,
            kind: PrincipalKind::Agent,
            is_anonymous: false,
            namespaces: crate::scope::NamespaceScope::unrestricted(),
            parent: Some(trogon_atlas_core::OwnerId::parse("alice").unwrap()),
        };
        assert!(!agent.authorizes_owner(&trogon_atlas_core::OwnerId::parse("bob").unwrap()));
    }
}
