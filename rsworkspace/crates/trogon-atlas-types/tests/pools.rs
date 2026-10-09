#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use trogon_atlas_types::{
    CompileLimits, CompileRunner, PoolError, PoolMember, PoolScope, ProtoPackagePrefix, ProtoPath,
    SourceBundle, SourceFile, TypeLibrarySource, TypePools,
};

fn member(slug: &str, prefix: &str, files: &[(&str, &str)], deps: &[&str]) -> PoolMember<String> {
    PoolMember {
        key: slug.to_owned(),
        source: TypeLibrarySource::new(
            ProtoPackagePrefix::parse(prefix).unwrap(),
            SourceBundle::new(
                files
                    .iter()
                    .map(|(p, c)| SourceFile::new(ProtoPath::parse(p).unwrap(), *c)),
                &CompileLimits::default(),
            )
            .unwrap(),
        ),
        dependencies: deps.iter().map(|d| (*d).to_owned()).collect(),
    }
}

fn pools() -> TypePools<String> {
    TypePools::new(CompileRunner::default(), CompileLimits::default())
}

fn scope(branch: Option<&str>) -> PoolScope {
    PoolScope::new("acme", branch)
}

fn money() -> PoolMember<String> {
    member(
        "money",
        "acme.money",
        &[(
            "acme/money/v1/money.proto",
            "syntax = \"proto3\";\npackage acme.money.v1;\nmessage Money { int64 cents = 1; }\n",
        )],
        &[],
    )
}

fn billing() -> PoolMember<String> {
    member(
        "billing",
        "acme.billing",
        &[(
            "acme/billing/v1/invoice.proto",
            "syntax = \"proto3\";\npackage acme.billing.v1;\nimport \"acme/money/v1/money.proto\";\nimport \"google/protobuf/timestamp.proto\";\nmessage Invoice { acme.money.v1.Money total = 1; google.protobuf.Timestamp issued_at = 2; }\n",
        )],
        &["money"],
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn builtins_carry_atlas_and_well_known_types() {
    let pools = pools();
    let pool = pools.get_or_build(scope(None), Vec::new()).await.unwrap();
    assert!(pool
        .get_message_by_name("google.protobuf.Timestamp")
        .is_some());
    assert!(pool.get_message_by_name("google.protobuf.Any").is_some());
    assert!(pool
        .all_messages()
        .any(|m| m.full_name().starts_with("trogonatlas.")));
}

#[tokio::test(flavor = "multi_thread")]
async fn dependents_resolve_types_from_their_dependencies() {
    let pools = pools();
    let pool = pools
        .get_or_build(scope(None), vec![billing(), money()])
        .await
        .unwrap();
    let invoice = pool.get_message_by_name("acme.billing.v1.Invoice").unwrap();
    let total = invoice.get_field_by_name("total").unwrap();
    assert_eq!(
        total.kind().as_message().unwrap().full_name(),
        "acme.money.v1.Money"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_dependency_missing_from_the_live_set_is_rejected() {
    let err = pools()
        .get_or_build(scope(None), vec![billing()])
        .await
        .unwrap_err();
    assert!(
        matches!(&err, PoolError::Dependency { library, .. } if library == "billing"),
        "{err}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn overlapping_package_prefixes_are_rejected() {
    let shadow = member(
        "shadow",
        "acme.money.v1",
        &[(
            "acme/shadow.proto",
            "syntax = \"proto3\";\npackage acme.money.v1;\nmessage Shadow {}\n",
        )],
        &[],
    );
    let err = pools()
        .get_or_build(scope(None), vec![money(), shadow])
        .await
        .unwrap_err();
    assert!(
        matches!(&err, PoolError::PrefixOverlap { first, second, .. } if first == "money" && second == "shadow"),
        "{err}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn pools_are_reused_until_the_live_set_changes() {
    let pools = pools();
    let first = pools
        .get_or_build(scope(None), vec![money(), billing()])
        .await
        .unwrap();
    let again = pools
        .get_or_build(scope(None), vec![billing(), money()])
        .await
        .unwrap();
    assert!(Arc::ptr_eq(&first, &again));

    let other_branch = pools
        .get_or_build(scope(Some("feature")), vec![money(), billing()])
        .await
        .unwrap();
    assert!(!Arc::ptr_eq(&first, &other_branch));

    let changed = member(
        "money",
        "acme.money",
        &[(
            "acme/money/v1/money.proto",
            "syntax = \"proto3\";\npackage acme.money.v1;\nmessage Money { int64 cents = 1; string currency = 2; }\n",
        )],
        &[],
    );
    let rebuilt = pools
        .get_or_build(scope(None), vec![changed, billing()])
        .await
        .unwrap();
    assert!(!Arc::ptr_eq(&first, &rebuilt));
    assert!(rebuilt
        .get_message_by_name("acme.money.v1.Money")
        .unwrap()
        .get_field_by_name("currency")
        .is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cached_pool_does_not_hide_a_dropped_dependency() {
    let pools = pools();
    pools
        .get_or_build(scope(None), vec![money(), billing()])
        .await
        .unwrap();

    let mut without_dependency = billing();
    without_dependency.dependencies.clear();
    let err = pools
        .get_or_build(scope(None), vec![money(), without_dependency])
        .await
        .unwrap_err();
    assert!(
        matches!(&err, PoolError::Compile { library, .. } if library == "billing"),
        "{err}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cached_pool_does_not_hide_a_changed_prefix() {
    let pools = pools();
    let shadow = |prefix: &str| {
        member(
            "shadow",
            prefix,
            &[(
                "acme/shadow/v1/shadow.proto",
                "syntax = \"proto3\";\npackage acme.shadow.v1;\nmessage Shadow {}\n",
            )],
            &[],
        )
    };
    pools
        .get_or_build(scope(None), vec![money(), shadow("acme.shadow")])
        .await
        .unwrap();

    let err = pools
        .get_or_build(scope(None), vec![money(), shadow("acme")])
        .await
        .unwrap_err();
    assert!(matches!(&err, PoolError::PrefixOverlap { .. }), "{err}");
}

#[tokio::test(flavor = "multi_thread")]
async fn least_recently_used_pools_are_evicted() {
    let pools: TypePools<String> =
        TypePools::with_capacity(CompileRunner::default(), CompileLimits::default(), 1);
    let trunk = pools
        .get_or_build(scope(None), vec![money()])
        .await
        .unwrap();
    pools
        .get_or_build(scope(Some("feature")), vec![money()])
        .await
        .unwrap();
    let rebuilt = pools
        .get_or_build(scope(None), vec![money()])
        .await
        .unwrap();
    assert!(!Arc::ptr_eq(&trunk, &rebuilt));
}
