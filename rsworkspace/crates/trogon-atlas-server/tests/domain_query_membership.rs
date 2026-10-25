#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::{collections::HashSet, sync::Arc};

use support::two_tenants::{
    authed, authed_branch, serve_with_registry, ACME_TOKEN, BETA_TOKEN, ROOT_TOKEN,
};
use tonic::transport::Channel;
use trogon_atlas_core::refs::{entity_id, entity_kind};
use trogon_atlas_proto::{self as pb, event_model_service_client::EventModelServiceClient};
use trogon_atlas_server::{
    auth::{RegistryFile, TokenRegistry},
    metrics::ServerMetrics,
    service::EventModelServiceImpl,
};

type Client = EventModelServiceClient<Channel>;

async fn client() -> Client {
    let store = Arc::new(trogon_atlas_testsupport::shared().await.store().await);
    let svc = EventModelServiceImpl::try_new(store, ServerMetrics::default()).unwrap();
    let registry: RegistryFile = toml::from_str(&format!(
        r#"
[principals.acme]
role = "writer"
tokens = ["{ACME_TOKEN}"]
parent = "acme"

[principals.beta]
role = "writer"
tokens = ["{BETA_TOKEN}"]
parent = "beta"

[principals.root]
role = "admin"
tokens = ["{ROOT_TOKEN}"]
"#
    ))
    .unwrap();
    serve_with_registry(svc, TokenRegistry::from_file_contents(registry).unwrap()).await
}

async fn namespace(c: &mut Client, name: &str, token: &str) -> String {
    c.register_namespace(authed(
        pb::RegisterNamespaceRequest {
            name: name.into(),
            parent: String::new(),
        },
        token,
    ))
    .await
    .unwrap()
    .into_inner()
    .namespace
    .unwrap()
    .id
}

fn id(ns: &str, slug: &str, version: u64) -> pb::Id {
    pb::Id {
        namespace: ns.into(),
        slug: slug.into(),
        version,
    }
}

fn domain(id: &pb::Id) -> pb::Entity {
    pb::Entity {
        kind: Some(pb::entity::Kind::Domain(pb::Domain {
            id: Some(id.clone()),
            title: "Domain".into(),
            ..Default::default()
        })),
        ..Default::default()
    }
}

fn subdomain(id: &pb::Id, domain: &pb::Id) -> pb::Entity {
    pb::Entity {
        kind: Some(pb::entity::Kind::Subdomain(pb::Subdomain {
            id: Some(id.clone()),
            title: "Subdomain".into(),
            domain: Some(pb::DomainRef {
                id: Some(domain.clone()),
            }),
            ..Default::default()
        })),
        ..Default::default()
    }
}

fn context(ns: &str, version: u64, realizes: &[pb::Id]) -> pb::Entity {
    pb::Entity {
        kind: Some(pb::entity::Kind::BoundedContext(pb::BoundedContext {
            id: Some(id(ns, ns, version)),
            title: "Context".into(),
            realizes: realizes
                .iter()
                .map(|id| pb::SubdomainRef {
                    id: Some(id.clone()),
                })
                .collect(),
            ..Default::default()
        })),
        ..Default::default()
    }
}

fn event(ns: &str, slug: &str, version: u64) -> pb::Entity {
    pb::Entity {
        kind: Some(pb::entity::Kind::Event(pb::Event {
            id: Some(id(ns, slug, version)),
            title: slug.into(),
            ..Default::default()
        })),
        ..Default::default()
    }
}

async fn put(c: &mut Client, entity: pb::Entity, branch: Option<&str>) {
    let body = pb::PutEntityRequest {
        entity: Some(entity),
        ..Default::default()
    };
    let req = match branch {
        Some(branch) => authed_branch(body, ROOT_TOKEN, branch),
        None => authed(body, ROOT_TOKEN),
    };
    c.put_entity(req).await.unwrap();
}

fn query(domain: &pb::Id) -> pb::ListEntitiesByDomainRequest {
    pb::ListEntitiesByDomainRequest {
        domain_id: Some(domain.clone()),
        ..Default::default()
    }
}

fn keys(entities: &[pb::Entity]) -> HashSet<pb::EntityKey> {
    entities
        .iter()
        .map(|e| pb::EntityKey::new(entity_kind(e).unwrap(), entity_id(e).unwrap()))
        .collect()
}

async fn listed(c: &mut Client, domain: &pb::Id) -> HashSet<pb::EntityKey> {
    keys(
        &c.list_entities_by_domain(authed(query(domain), ROOT_TOKEN))
            .await
            .unwrap()
            .into_inner()
            .entities,
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn domain_and_subdomain_membership_preserve_exact_versions() {
    let mut c = client().await;
    let ns = namespace(&mut c, "strategy", ROOT_TOKEN).await;
    let d1 = id(&ns, "domain", 1);
    let d2 = id(&ns, "domain", 2);
    let s1 = id(&ns, "subdomain", 1);
    let s2 = id(&ns, "subdomain", 2);
    let expected = vec![domain(&d1), subdomain(&s1, &d1)];
    for e in expected
        .iter()
        .cloned()
        .chain([domain(&d2), subdomain(&s2, &d2)])
    {
        put(&mut c, e, None).await;
    }
    assert_eq!(listed(&mut c, &d1).await, keys(&expected));
    assert_eq!(
        listed(&mut c, &d2).await,
        keys(&[domain(&d2), subdomain(&s2, &d2)])
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn context_uses_its_own_exact_realizes_not_other_context_versions() {
    let mut c = client().await;
    let ns = namespace(&mut c, "strategy", ROOT_TOKEN).await;
    let bc = namespace(&mut c, "implementation", ROOT_TOKEN).await;
    let d1 = id(&ns, "one", 1);
    let d2 = id(&ns, "two", 1);
    let s1 = id(&ns, "subdomain", 1);
    let s2 = id(&ns, "subdomain", 2);
    for e in [
        domain(&d1),
        domain(&d2),
        subdomain(&s1, &d1),
        subdomain(&s2, &d2),
        context(&bc, 1, std::slice::from_ref(&s1)),
        context(&bc, 2, std::slice::from_ref(&s2)),
        event(&bc, "ordinary", 7),
    ] {
        put(&mut c, e, None).await;
    }
    for (d, s, version) in [(&d1, &s1, 1), (&d2, &s2, 2)] {
        assert_eq!(
            listed(&mut c, d).await,
            keys(&[
                domain(d),
                subdomain(s, d),
                context(&bc, version, std::slice::from_ref(s)),
                event(&bc, "ordinary", 7)
            ])
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn all_realized_subdomains_contribute_and_shared_membership_is_unique() {
    let mut c = client().await;
    let ns = namespace(&mut c, "strategy", ROOT_TOKEN).await;
    let bc = namespace(&mut c, "implementation", ROOT_TOKEN).await;
    let other_bc = namespace(&mut c, "other", ROOT_TOKEN).await;
    let d1 = id(&ns, "one", 1);
    let d2 = id(&ns, "two", 1);
    let s1 = id(&ns, "one", 1);
    let s2 = id(&ns, "two", 1);
    let realizes = [s1.clone(), s2.clone(), s2.clone()];
    let dangling = [id(&ns, "missing", 9), s2.clone()];
    for e in [
        domain(&d1),
        domain(&d2),
        subdomain(&s1, &d1),
        subdomain(&s2, &d2),
        context(&bc, 1, &realizes),
        context(&other_bc, 1, &dangling),
        event(&bc, "ordinary", 3),
        event(&other_bc, "ordinary", 5),
    ] {
        put(&mut c, e, None).await;
    }
    let actual = c
        .list_entities_by_domain(authed(query(&d2), ROOT_TOKEN))
        .await
        .unwrap()
        .into_inner()
        .entities;
    assert_eq!(
        keys(&actual),
        keys(&[
            domain(&d2),
            subdomain(&s2, &d2),
            context(&bc, 1, &realizes),
            context(&other_bc, 1, &dangling),
            event(&bc, "ordinary", 3),
            event(&other_bc, "ordinary", 5)
        ])
    );
    assert_eq!(actual.len(), keys(&actual).len());
}

#[tokio::test(flavor = "multi_thread")]
async fn latest_matching_versions_are_selected_before_kind_filtered_pagination() {
    let mut c = client().await;
    let ns = namespace(&mut c, "strategy", ROOT_TOKEN).await;
    let bc = namespace(&mut c, "implementation", ROOT_TOKEN).await;
    let d1 = id(&ns, "domain", 1);
    let d2 = id(&ns, "domain", 2);
    let s1 = id(&ns, "subdomain", 1);
    let s2 = id(&ns, "subdomain", 2);
    for e in [
        domain(&d1),
        domain(&d2),
        subdomain(&s1, &d1),
        subdomain(&s2, &d2),
        context(&bc, 1, std::slice::from_ref(&s1)),
        context(&bc, 2, std::slice::from_ref(&s2)),
        event(&bc, "alpha", 2),
        event(&bc, "alpha", 10),
        event(&bc, "beta", 3),
    ] {
        put(&mut c, e, None).await;
    }
    let mut q = query(&d1);
    q.latest_versions_only = true;
    let all = c
        .list_entities_by_domain(authed(q.clone(), ROOT_TOKEN))
        .await
        .unwrap()
        .into_inner()
        .entities;
    assert_eq!(
        keys(&all),
        keys(&[
            domain(&d1),
            subdomain(&s1, &d1),
            context(&bc, 1, std::slice::from_ref(&s1)),
            event(&bc, "alpha", 10),
            event(&bc, "beta", 3)
        ])
    );
    q.kinds = vec![pb::EntityKind::Event as i32];
    q.page_size = 1;
    let first = c
        .list_entities_by_domain(authed(q.clone(), ROOT_TOKEN))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(keys(&first.entities), keys(&[event(&bc, "alpha", 10)]));
    assert_ne!(first.next_page_token, "");
    q.page_token = first.next_page_token;
    let second = c
        .list_entities_by_domain(authed(q, ROOT_TOKEN))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(keys(&second.entities), keys(&[event(&bc, "beta", 3)]));
    assert_eq!(second.next_page_token, "");
}

#[tokio::test(flavor = "multi_thread")]
async fn latest_flag_deduplicates_versions_without_affecting_unfiltered_results() {
    let mut c = client().await;
    let ns = namespace(&mut c, "strategy", ROOT_TOKEN).await;
    let bc = namespace(&mut c, "implementation", ROOT_TOKEN).await;
    let d = id(&ns, "domain", 1);
    let s = id(&ns, "subdomain", 1);
    for e in [
        domain(&d),
        subdomain(&s, &d),
        context(&bc, 1, &[s]),
        event(&bc, "alpha", 2),
        event(&bc, "alpha", 10),
        event(&bc, "beta", 3),
    ] {
        put(&mut c, e, None).await;
    }
    let mut q = query(&d);
    q.kinds = vec![pb::EntityKind::Event as i32];
    let unfiltered = c
        .list_entities_by_domain(authed(q.clone(), ROOT_TOKEN))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        keys(&unfiltered.entities),
        keys(&[
            event(&bc, "alpha", 2),
            event(&bc, "alpha", 10),
            event(&bc, "beta", 3)
        ])
    );
    q.latest_versions_only = true;
    q.page_size = 1;
    let first = c
        .list_entities_by_domain(authed(q.clone(), ROOT_TOKEN))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(keys(&first.entities), keys(&[event(&bc, "alpha", 10)]));
    assert_ne!(first.next_page_token, "");
    q.page_token = first.next_page_token;
    let second = c
        .list_entities_by_domain(authed(q, ROOT_TOKEN))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(keys(&second.entities), keys(&[event(&bc, "beta", 3)]));
    assert_eq!(second.next_page_token, "");
}

#[tokio::test(flavor = "multi_thread")]
async fn foreign_branch_cannot_disclose_staged_entities_owned_by_the_caller() {
    let mut c = client().await;
    let ns = namespace(&mut c, "strategy", BETA_TOKEN).await;
    let bc = namespace(&mut c, "implementation", BETA_TOKEN).await;
    let d = id(&ns, "domain", 1);
    let s = id(&ns, "subdomain", 1);
    let baseline = [
        domain(&d),
        subdomain(&s, &d),
        context(&bc, 1, std::slice::from_ref(&s)),
    ];
    for entity in &baseline {
        put(&mut c, entity.clone(), None).await;
    }
    let staged = event(&bc, "unmerged", 1);
    let mut expected = baseline.to_vec();
    expected.push(staged.clone());
    for branch in ["@acme:membership", "@beta:membership"] {
        c.create_branch(authed(
            pb::CreateBranchRequest {
                name: branch.into(),
                doc: String::new(),
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap();
        put(&mut c, staged.clone(), Some(branch)).await;
        let root = c
            .list_entities_by_domain(authed_branch(query(&d), ROOT_TOKEN, branch))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(keys(&root.entities), keys(&expected));
    }
    let beta_baseline = c
        .list_entities_by_domain(authed(query(&d), BETA_TOKEN))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(keys(&beta_baseline.entities), keys(&baseline));
    let beta_branch = c
        .list_entities_by_domain(authed_branch(query(&d), BETA_TOKEN, "@beta:membership"))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(keys(&beta_branch.entities), keys(&expected));

    let denied = c
        .list_entities_by_domain(authed_branch(query(&d), BETA_TOKEN, "@acme:membership"))
        .await
        .expect_err("Beta must not read its unmerged entities through Acme's branch");
    assert_eq!(denied.code(), tonic::Code::NotFound);
    assert_eq!(denied.message(), "branch not found");
}

#[tokio::test(flavor = "multi_thread")]
async fn branch_overlay_and_owner_visibility_bound_membership() {
    let mut c = client().await;
    let ns = namespace(&mut c, "strategy", ACME_TOKEN).await;
    let bc = namespace(&mut c, "implementation", ACME_TOKEN).await;
    let hidden = namespace(&mut c, "hidden", BETA_TOKEN).await;
    let d = id(&ns, "domain", 1);
    let s = id(&ns, "subdomain", 1);
    for e in [
        domain(&d),
        subdomain(&s, &d),
        context(&bc, 1, std::slice::from_ref(&s)),
        context(&hidden, 1, std::slice::from_ref(&s)),
        event(&hidden, "private", 1),
    ] {
        put(&mut c, e, None).await;
    }
    c.create_branch(authed(
        pb::CreateBranchRequest {
            name: "@acme:membership".into(),
            doc: String::new(),
        },
        ROOT_TOKEN,
    ))
    .await
    .unwrap();
    put(
        &mut c,
        event(&bc, "branch-only", 4),
        Some("@acme:membership"),
    )
    .await;
    let baseline = c
        .list_entities_by_domain(authed(query(&d), ACME_TOKEN))
        .await
        .unwrap()
        .into_inner()
        .entities;
    let expected = [
        domain(&d),
        subdomain(&s, &d),
        context(&bc, 1, std::slice::from_ref(&s)),
    ];
    assert_eq!(keys(&baseline), keys(&expected));
    let branch = c
        .list_entities_by_domain(authed_branch(query(&d), ACME_TOKEN, "@acme:membership"))
        .await
        .unwrap()
        .into_inner()
        .entities;
    let mut expected = expected.to_vec();
    expected.push(event(&bc, "branch-only", 4));
    assert_eq!(keys(&branch), keys(&expected));

    put(&mut c, context(&bc, 1, &[]), Some("@acme:membership")).await;
    let disconnected = c
        .list_entities_by_domain(authed_branch(query(&d), ACME_TOKEN, "@acme:membership"))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        keys(&disconnected.entities),
        keys(&[domain(&d), subdomain(&s, &d)])
    );
    let unchanged = c
        .list_entities_by_domain(authed(query(&d), ACME_TOKEN))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(keys(&unchanged.entities), keys(&baseline));

    let hidden = c
        .list_entities_by_domain(authed(query(&d), BETA_TOKEN))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(hidden.entities, Vec::<pb::Entity>::new());
}
