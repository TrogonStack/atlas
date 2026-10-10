//! Shared fixture for every cross-tenant isolation test: two tenants, acme
//! and beta, who each register a namespace under the *same* bare name and
//! use the *same* slugs, each writing a canary string only its own token
//! should ever be able to read back.
//!
//! `tests/cross_tenant_reads.rs` already proves the boundary when beta owns
//! nothing under the name acme used. That leaves the harder case untested:
//! `RegisterNamespace` documents a name as unique only "within the caller's
//! parent", so acme and beta can each hold a namespace literally named
//! `tt-shared`, minted to different ids. A leak here cannot be mistaken for
//! "beta was never going to see that name anyway": beta really does have an
//! entity at that exact name and slug, under a different id, and the only
//! question is whether some read path keys off the display name instead of
//! the id and hands back acme's.
//!
//! This module only builds fixtures and servers. The assertions live in
//! `tests/two_tenants.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used, dead_code)]

use std::sync::Arc;

use tokio::net::TcpListener;
use tonic::{
    metadata::MetadataValue, service::interceptor::InterceptedService, transport::Channel, Code,
    Status,
};
use trogon_atlas_authzed::{PresharedKey, SpiceDbConfig};
use trogon_atlas_proto as pb;
use trogon_atlas_proto::{
    event_model_service_client::EventModelServiceClient,
    event_model_service_server::EventModelServiceServer,
};
use trogon_atlas_server::{
    auth::{AuthzLayer, BearerAuth, TokenRegistry},
    service::EventModelServiceImpl,
    spicedb::{Freshness, SpiceDbAuthorizer},
};
use trogon_atlas_store::Store;

pub const ACME_TOKEN: &str = "tt-acme-token";
pub const BETA_TOKEN: &str = "tt-beta-token";

/// Bound to an owner that never registers or writes anything. Stands in for
/// a tenant whose lens admits no namespace at all, so every record in the
/// global change log is invisible to it -- the case a leaking `next_token`
/// would otherwise expose.
pub const GAMMA_TOKEN: &str = "tt-gamma-token";

/// Bound to no owner, admin role: the only principal allowed to create a
/// branch, since branch metadata is not partitioned by owner and the server
/// refuses to show it to a caller bound to one (see `refuse_unpartitioned`
/// in `src/service.rs`). Standing in for the "local" operator principal a
/// real deployment uses for exactly this kind of setup action.
pub const ROOT_TOKEN: &str = "tt-root-token";

/// The bare display name both tenants register, each under their own owner.
/// `RegisterNamespace` only requires a name be unused *within the caller's
/// own parent*, so both registrations succeed and mint different ids.
pub const NS: &str = "tt-shared";

pub const EVENT_SLUG: &str = "placed";
pub const SLICE_SLUG: &str = "place";
pub const STORYBOARD_SLUG: &str = "checkout";
pub const MODEL_SLUG: &str = "model";

/// A string unique to one tenant that must never show up in a response
/// served to the other. Baked into every title/doc field an entity has, so
/// the sweep in `tests/two_tenants.rs` has one needle to search for no
/// matter which RPC it is checking.
pub fn canary(owner: &str) -> String {
    format!("canary-{owner}-do-not-leak")
}

/// A token-file registry binding `acme-writer`/`beta-writer` to the owners
/// `acme`/`beta`, plus an unbound `root` admin for branch setup.
pub fn make_registry() -> TokenRegistry {
    let toml = format!(
        r#"
[principals.acme-writer]
role = "writer"
tokens = ["{ACME_TOKEN}"]
parent = "acme"

[principals.beta-writer]
role = "writer"
tokens = ["{BETA_TOKEN}"]
parent = "beta"

[principals.root]
role = "admin"
tokens = ["{ROOT_TOKEN}"]

[principals.gamma-writer]
role = "writer"
tokens = ["{GAMMA_TOKEN}"]
parent = "gamma"
"#
    );
    let file: trogon_atlas_server::auth::RegistryFile =
        toml::from_str(&toml).expect("valid registry TOML");
    TokenRegistry::from_file_contents(file).expect("valid registry")
}

/// Serve a pre-built service behind `BearerAuth` with the given registry,
/// the way production wires it, and hand back a connected client.
pub async fn serve_with_registry(
    svc: EventModelServiceImpl,
    registry: TokenRegistry,
) -> EventModelServiceClient<Channel> {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let grpc = EventModelServiceServer::new(svc);
    let auth = BearerAuth::new(Arc::new(registry));
    let data_plane = InterceptedService::new(
        tower::ServiceBuilder::new().layer(AuthzLayer).service(grpc),
        auth,
    );
    tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(data_plane)
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
            .unwrap();
    });
    EventModelServiceClient::connect(format!("http://{addr}"))
        .await
        .unwrap()
}

/// A server over a fresh, isolated store under the default token-registry
/// authorizer: acme and beta tokens bound by `parent` alone.
pub async fn client() -> EventModelServiceClient<Channel> {
    let store: Arc<dyn Store> = Arc::new(trogon_atlas_testsupport::shared().await.store().await);
    let svc = EventModelServiceImpl::try_new(store).unwrap();
    serve_with_registry(svc, make_registry()).await
}

pub fn authed<T>(body: T, token: &str) -> tonic::Request<T> {
    let mut req = tonic::Request::new(body);
    req.metadata_mut().insert(
        "authorization",
        MetadataValue::try_from(format!("Bearer {token}")).unwrap(),
    );
    req
}

pub fn authed_branch<T>(body: T, token: &str, branch: &str) -> tonic::Request<T> {
    let mut req = authed(body, token);
    req.metadata_mut().insert(
        "x-trogon-atlas-branch",
        MetadataValue::try_from(branch).expect("branch name is valid metadata"),
    );
    req
}

/// Claim `NS` for `token`'s owner and return the minted namespace id.
/// Called once per tenant so both can hold a namespace literally named
/// [`NS`] at the same time, each with its own id.
pub async fn register_ns(c: &mut EventModelServiceClient<Channel>, token: &str) -> String {
    c.register_namespace(authed(
        pb::RegisterNamespaceRequest {
            name: NS.into(),
            parent: String::new(),
        },
        token,
    ))
    .await
    .expect("register namespace")
    .into_inner()
    .namespace
    .expect("response carries the namespace it registered")
    .id
}

pub fn id(ns: &str, slug: &str) -> pb::Id {
    pb::Id {
        namespace: ns.into(),
        slug: slug.into(),
        version: 1,
    }
}

pub fn event_entity(ns: &str, slug: &str, canary: &str) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::Event(pb::Event {
            id: Some(id(ns, slug)),
            title: format!("{slug}-{canary}"),
            ..Default::default()
        })),
    }
}

/// An event with a caller-chosen title, for search tests that need exact
/// control over term frequency rather than the fixed `{slug}-{canary}` shape
/// `event_entity` bakes in.
pub fn event_entity_titled(ns: &str, slug: &str, title: &str) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::Event(pb::Event {
            id: Some(id(ns, slug)),
            title: title.into(),
            ..Default::default()
        })),
    }
}

/// A command slice that emits `event_slug`, so the graph has one edge to
/// walk and the projections have something to project.
pub fn slice_entity(ns: &str, slug: &str, event_slug: &str, canary: &str) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::CommandSlice(pb::CommandSlice {
            id: Some(id(ns, slug)),
            title: format!("{slug}-{canary}"),
            emitted_events: vec![pb::EventEdge {
                event: Some(pb::EventRef {
                    id: Some(id(ns, event_slug)),
                }),
                ..Default::default()
            }],
            ..Default::default()
        })),
    }
}

pub fn storyboard_entity(ns: &str, slug: &str, slice_slug: &str, canary: &str) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::Storyboard(pb::Storyboard {
            id: Some(id(ns, slug)),
            title: format!("{slug}-{canary}"),
            slices: vec![pb::SliceRef {
                id: Some(id(ns, slice_slug)),
            }],
            ..Default::default()
        })),
    }
}

pub fn event_model_entity(ns: &str, slug: &str, canary: &str) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::EventModel(pb::EventModel {
            id: Some(id(ns, slug)),
            title: format!("{slug}-{canary}"),
            members: vec![
                pb::EntityRef {
                    kind: pb::EntityKind::CommandSlice as i32,
                    id: Some(id(ns, SLICE_SLUG)),
                },
                pb::EntityRef {
                    kind: pb::EntityKind::Event as i32,
                    id: Some(id(ns, EVENT_SLUG)),
                },
            ],
            ..Default::default()
        })),
    }
}

pub fn put_req(entity: pb::Entity) -> pb::PutEntityRequest {
    pb::PutEntityRequest {
        entity: Some(entity),
        create_only: false,
        if_match: String::new(),
        validate_only: false,
        force: false,
        operation_id: String::new(),
    }
}

pub fn eref(ns: &str, kind: pb::EntityKind, slug: &str) -> pb::EntityRef {
    pb::EntityRef {
        kind: kind as i32,
        id: Some(id(ns, slug)),
    }
}

/// Register `NS` for `token`'s owner and write its full graph (event ->
/// slice -> storyboard -> event model) under that minted id, with this
/// tenant's own canary baked into every title. Returns the minted id.
pub async fn write_world(
    c: &mut EventModelServiceClient<Channel>,
    token: &str,
    owner: &str,
) -> String {
    let ns = register_ns(c, token).await;
    let mark = canary(owner);
    c.put_entity(authed(put_req(event_entity(&ns, EVENT_SLUG, &mark)), token))
        .await
        .expect("writes its event");
    c.put_entity(authed(
        put_req(slice_entity(&ns, SLICE_SLUG, EVENT_SLUG, &mark)),
        token,
    ))
    .await
    .expect("writes its slice");
    c.put_entity(authed(
        put_req(storyboard_entity(&ns, STORYBOARD_SLUG, SLICE_SLUG, &mark)),
        token,
    ))
    .await
    .expect("writes its storyboard");
    c.put_entity(authed(
        put_req(event_model_entity(&ns, MODEL_SLUG, &mark)),
        token,
    ))
    .await
    .expect("writes its event model");
    ns
}

/// Both tenants' identically-named, identically-shaped graphs, each under
/// its own minted id and its own canary.
pub struct World {
    pub client: EventModelServiceClient<Channel>,
    pub acme_ns: String,
    pub beta_ns: String,
}

pub async fn world() -> World {
    let mut c = client().await;
    let acme_ns = write_world(&mut c, ACME_TOKEN, "acme").await;
    let beta_ns = write_world(&mut c, BETA_TOKEN, "beta").await;
    assert_ne!(
        acme_ns, beta_ns,
        "two owners registering the same name must mint different ids",
    );
    World {
        client: c,
        acme_ns,
        beta_ns,
    }
}

/// Branch metadata is not partitioned by owner, so only the unbound `root`
/// principal may create one; see [`ROOT_TOKEN`].
pub async fn create_branch(c: &mut EventModelServiceClient<Channel>, name: &str) {
    c.create_branch(authed(
        pb::CreateBranchRequest {
            name: name.into(),
            doc: String::new(),
        },
        ROOT_TOKEN,
    ))
    .await
    .expect("create branch");
}

/// Tokens and minted namespace ids for a `SpiceDbAuthorizer`-backed world: a
/// registry binding each tenant's token to its owner (the `parent` field a
/// request's `Visibility` is built from), with that owner's SpiceDB
/// membership established by `reconcile` the same way a deployment's
/// periodic sync would.
pub struct SpiceDbWorld {
    pub client: EventModelServiceClient<Channel>,
    pub acme_token: String,
    pub beta_token: String,
    pub acme_ns: String,
    pub beta_ns: String,
    // Held for as long as the world is: see `SPICEDB_SERIALIZED` below.
    _spicedb_serialized: tokio::sync::MutexGuard<'static, ()>,
}

/// One `_under_spicedb` test at a time against the shared container.
///
/// SpiceDB's in-memory datastore does not hold up its own read-your-writes
/// guarantee under concurrent load: a `FullyConsistent` check can still miss
/// a relationship whose write already returned successfully to the same
/// caller, when another connection is concurrently writing and checking
/// against the same container (`tests/spicedb_authorizer.rs`
/// hit and documented the same effect). Object ids unique per `tag` isolate
/// the tenants' *data* from each other; they do nothing to stop one test's
/// writes from perturbing another's concurrently-evaluated checks. Holding
/// this for a world's whole lifetime removes that interleaving instead of
/// retrying around it.
static SPICEDB_SERIALIZED: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// As [`world`], but authorized through a real SpiceDB rather than the
/// token file's `parent` column alone. `tag` must be unique per test: the
/// container behind `shared_spicedb()` lives for the whole test binary, so
/// two tests sharing one tag would see each other's relationships.
pub async fn spicedb_world(tag: &str) -> SpiceDbWorld {
    let spicedb_serialized = SPICEDB_SERIALIZED.lock().await;
    let store: Arc<dyn Store> = Arc::new(trogon_atlas_testsupport::shared().await.store().await);
    let spicedb = trogon_atlas_testsupport::shared_spicedb().await;
    let config = SpiceDbConfig::new(
        spicedb.endpoint.clone(),
        PresharedKey::parse(trogon_atlas_testsupport::SPICEDB_PRESHARED_KEY).unwrap(),
    );
    // The authorizer must read the same `NamespaceDirectory` the service
    // invalidates on every claim (`svc.directory()`, mirroring how
    // `main.rs` wires `build_spicedb`), not a directory of its own: a
    // second, disconnected instance never learns about a namespace this
    // service just registered and stays stuck at `Unclaimed`, which sends
    // every write back through claim-on-first-write for a name that is
    // already somebody else's.
    let svc_shell = EventModelServiceImpl::try_new(store.clone()).unwrap();
    let authorizer = SpiceDbAuthorizer::new(
        &config,
        svc_shell.directory(),
        store.clone(),
        Freshness::FullyConsistent,
    )
    .unwrap();

    let acme_name = format!("tt-{tag}-acme-writer");
    let beta_name = format!("tt-{tag}-beta-writer");
    let acme_token = format!("tt-{tag}-acme-token");
    let beta_token = format!("tt-{tag}-beta-token");
    let acme_owner = format!("tt-{tag}-acme");
    let beta_owner = format!("tt-{tag}-beta");

    // Installs the schema on first use and, every time, makes SpiceDB agree
    // that each principal belongs to its owner -- the relationship
    // `on_registered`/`admits` actually check, which the registry's
    // `parent` field alone does not establish under this authorizer.
    authorizer
        .reconcile(&[
            (
                Arc::from(acme_name.as_str()),
                trogon_atlas_core::OwnerId::parse(&acme_owner).unwrap(),
            ),
            (
                Arc::from(beta_name.as_str()),
                trogon_atlas_core::OwnerId::parse(&beta_owner).unwrap(),
            ),
        ])
        .await
        .expect("schema install and membership sync");

    let toml = format!(
        r#"
[principals.{acme_name}]
role = "writer"
tokens = ["{acme_token}"]
parent = "{acme_owner}"

[principals.{beta_name}]
role = "writer"
tokens = ["{beta_token}"]
parent = "{beta_owner}"
"#
    );
    let file: trogon_atlas_server::auth::RegistryFile =
        toml::from_str(&toml).expect("valid registry TOML");
    let registry = TokenRegistry::from_file_contents(file).expect("valid registry");

    let svc = svc_shell.with_authorizer(Arc::new(authorizer));
    let mut client = serve_with_registry(svc, registry).await;

    let acme_ns = write_world(&mut client, &acme_token, "acme").await;
    let beta_ns = write_world(&mut client, &beta_token, "beta").await;
    assert_ne!(
        acme_ns, beta_ns,
        "two owners registering the same name must mint different ids",
    );

    SpiceDbWorld {
        client,
        acme_token,
        beta_token,
        acme_ns,
        beta_ns,
        _spicedb_serialized: spicedb_serialized,
    }
}

/// A response must not contain the other tenant's canary, whether it is a
/// hit on this tenant's own graph, an empty result, or `NotFound`.
/// `PermissionDenied` is treated as a leak too: on a name both tenants hold,
/// it would tell beta that a *specific* slug exists versus one that does
/// not, which is exactly the directory a shared bare name must not become.
pub fn assert_no_leak<T: std::fmt::Debug>(
    what: &str,
    result: Result<tonic::Response<T>, Status>,
    forbidden: &str,
) {
    match result {
        Ok(resp) => {
            let body = resp.into_inner();
            let debug = format!("{body:?}");
            assert!(
                !debug.contains(forbidden),
                "{what} leaked another tenant's canary: {debug}",
            );
        }
        Err(status) => {
            assert_ne!(
                status.code(),
                Code::PermissionDenied,
                "{what} answered PermissionDenied on a name both tenants hold, \
                 which tells beta the slug exists under acme too: {}",
                status.message(),
            );
        }
    }
}
