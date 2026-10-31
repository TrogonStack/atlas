//! In-process gRPC integration tests for `ExportAttestedModel`: a bundle
//! names one state, verifies offline, fails verification once any normative
//! field changes, and is refused or marked when it cannot cover its scope.
//!
//! The store is shared across tests in this file, so every test writes into
//! its own namespace.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use tokio::net::TcpListener;
use tonic::{metadata::MetadataValue, transport::Channel};
use trogon_atlas_core::{
    attested_bundle::{
        self, Authenticity, BundleError, Completeness, IncompleteReason, UnresolvedCause,
    },
    attested_signing::{SigningKey, TrustedKeys},
};
use trogon_atlas_proto as pb;
use trogon_atlas_proto::{
    event_model_service_client::EventModelServiceClient,
    event_model_service_server::EventModelServiceServer,
};
use trogon_atlas_server::{
    attested_signing::SigningKeySource, metrics::ServerMetrics, service::EventModelServiceImpl,
};

async fn start_server(snapshot_cap: Option<usize>) -> String {
    let store = trogon_atlas_testsupport::shared().await.store().await;
    let mut svc =
        EventModelServiceImpl::try_new(Arc::new(store), ServerMetrics::default()).unwrap();
    if let Some(cap) = snapshot_cap {
        svc = svc.with_snapshot_cap(cap);
    }
    serve(svc).await
}

async fn serve(svc: EventModelServiceImpl) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let grpc = EventModelServiceServer::new(svc);
    tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(grpc)
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
            .unwrap();
    });
    format!("http://{addr}")
}

async fn client(endpoint: &str) -> EventModelServiceClient<Channel> {
    EventModelServiceClient::connect(endpoint.to_string())
        .await
        .unwrap()
}

fn id(ns: &str, slug: &str) -> pb::Id {
    pb::Id {
        namespace: ns.into(),
        slug: slug.into(),
        version: 1,
    }
}

fn event(ns: &str, slug: &str) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::Event(pb::Event {
            id: Some(id(ns, slug)),
            title: format!("{slug} happened"),
            ..Default::default()
        })),
    }
}

fn event_model(ns: &str, slug: &str, title: &str, members: &[&str]) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::EventModel(pb::EventModel {
            id: Some(id(ns, slug)),
            title: title.into(),
            members: members
                .iter()
                .map(|m| pb::EntityRef {
                    kind: pb::EntityKind::Event as i32,
                    id: Some(id(ns, m)),
                })
                .collect(),
            ..Default::default()
        })),
    }
}

fn put_req(entity: pb::Entity) -> pb::PutEntityRequest {
    pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(entity),
        create_only: false,
        if_match: String::new(),
        validate_only: false,
        force: false,
    }
}

async fn put(c: &mut EventModelServiceClient<Channel>, entity: pb::Entity) {
    c.put_entity(put_req(entity)).await.unwrap();
}

fn export_req(ns: &str, slug: &str) -> pb::ExportAttestedModelRequest {
    pb::ExportAttestedModelRequest {
        id: Some(id(ns, slug)),
        as_of_changeset: String::new(),
        allow_incomplete: false,
    }
}

async fn export(
    c: &mut EventModelServiceClient<Channel>,
    req: pb::ExportAttestedModelRequest,
) -> pb::AttestedModelBundle {
    c.export_attested_model(req)
        .await
        .unwrap()
        .into_inner()
        .bundle
        .unwrap()
}

async fn newest_changeset(c: &mut EventModelServiceClient<Channel>) -> String {
    c.list_changesets(pb::ListChangesetsRequest {
        page_token: String::new(),
        page_size: 1,
        branch: String::new(),
        namespace: String::new(),
    })
    .await
    .unwrap()
    .into_inner()
    .changesets[0]
        .id
        .clone()
}

fn entry_slugs(bundle: &pb::AttestedModelBundle) -> Vec<String> {
    bundle
        .entries
        .iter()
        .map(|e| {
            e.entity
                .as_ref()
                .and_then(|r| r.id.as_ref())
                .unwrap()
                .slug
                .clone()
        })
        .collect()
}

fn mentions(issues: &[pb::ValidationIssue], slug: &str) -> bool {
    issues.iter().any(|i| {
        i.subject
            .as_ref()
            .and_then(|s| s.id.as_ref())
            .is_some_and(|id| id.slug == slug)
    })
}

#[tokio::test(flavor = "multi_thread")]
async fn complete_export_verifies_and_names_the_current_state() {
    let endpoint = start_server(None).await;
    let mut c = client(&endpoint).await;
    let ns = "attested-current";
    put(&mut c, event(ns, "placed")).await;
    put(&mut c, event_model(ns, "checkout", "Checkout", &["placed"])).await;
    let boundary = newest_changeset(&mut c).await;

    let bundle = export(&mut c, export_req(ns, "checkout")).await;

    let verified = attested_bundle::verify(&bundle).unwrap();
    assert_eq!(verified.completeness, Completeness::Complete);
    assert_eq!(verified.digest.to_hex(), bundle.bundle_digest);
    assert_eq!(bundle.format, attested_bundle::BUNDLE_FORMAT);
    assert_eq!(verified.authenticity, Authenticity::Unsigned);
    let state = bundle.state.as_ref().unwrap();
    assert_eq!(state.mode(), pb::attested_state::Mode::Current);
    assert_eq!(state.baseline_changeset_id, boundary);
    assert_eq!(state.branch_changeset_id, "");
    let scope = bundle.scope.as_ref().unwrap();
    assert_eq!(scope.event_model_id, Some(id(ns, "checkout")));
    assert!(scope.lens.as_ref().unwrap().unrestricted);
    assert_eq!(entry_slugs(&bundle), ["placed", "checkout"]);
    assert!(bundle.manifests.contains("Checkout"));
    let validation = bundle.validation.as_ref().unwrap();
    assert_eq!(
        validation.rule_catalog.as_ref().unwrap().digest,
        attested_bundle::rule_catalog_digest(
            &c.list_validation_rules(pb::ListValidationRulesRequest {})
                .await
                .unwrap()
                .into_inner()
                .rules
        )
        .unwrap()
        .to_hex()
    );

    let json = attested_bundle::bundle_to_json(&bundle).unwrap();
    let reread = attested_bundle::bundle_from_json(&json).unwrap();
    assert_eq!(attested_bundle::verify(&reread).unwrap(), verified);
}

#[tokio::test(flavor = "multi_thread")]
async fn changing_any_normative_field_fails_verification() {
    let endpoint = start_server(None).await;
    let mut c = client(&endpoint).await;
    let ns = "attested-tamper";
    put(&mut c, event(ns, "placed")).await;
    put(&mut c, event_model(ns, "checkout", "Checkout", &["placed"])).await;
    let bundle = export(&mut c, export_req(ns, "checkout")).await;
    assert!(
        !bundle.validation.as_ref().unwrap().issues.is_empty(),
        "the fixture needs a finding to tamper with"
    );

    type Tamper = fn(&mut pb::AttestedModelBundle);
    let tampers: [(&str, Tamper); 8] = [
        ("manifests", |b| b.manifests.push_str("# edited\n")),
        ("external manifests", |b| {
            b.external_manifests.push_str("# edited\n");
        }),
        ("finding", |b| {
            b.validation.as_mut().unwrap().issues[0].message = "nothing to see".into();
        }),
        ("dropped finding", |b| {
            b.validation.as_mut().unwrap().issues.remove(0);
        }),
        ("coverage evidence", |b| {
            let coverage = b.validation.as_mut().unwrap().coverage.as_mut().unwrap();
            coverage.checks[0].subjects_examined += 1;
        }),
        ("entry", |b| {
            b.entries[0].content_hash = "0".repeat(64);
        }),
        ("scope", |b| {
            b.scope.as_mut().unwrap().branch = "elsewhere".into();
        }),
        ("rule catalogue", |b| {
            let catalog = b
                .validation
                .as_mut()
                .unwrap()
                .rule_catalog
                .as_mut()
                .unwrap();
            catalog.server_version = "0.0.0-older".into();
        }),
    ];
    for (field, tamper) in tampers {
        let mut tampered = bundle.clone();
        tamper(&mut tampered);
        assert!(
            matches!(
                attested_bundle::verify(&tampered),
                Err(BundleError::DigestMismatch { .. })
            ),
            "editing the {field} must fail the digest"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn incomplete_export_is_refused_unless_explicitly_allowed() {
    let endpoint = start_server(Some(1)).await;
    let mut c = client(&endpoint).await;
    let ns = "attested-capped";
    put(&mut c, event(ns, "placed")).await;
    put(&mut c, event(ns, "shipped")).await;
    put(
        &mut c,
        event_model(ns, "checkout", "Checkout", &["placed", "shipped"]),
    )
    .await;

    let refused = c
        .export_attested_model(export_req(ns, "checkout"))
        .await
        .unwrap_err();
    assert_eq!(refused.code(), tonic::Code::FailedPrecondition);
    let failure = trogon_atlas_client::failure::Failure::from_status(&refused);
    assert_eq!(failure.code.unwrap().as_str(), "EXPORT_INCOMPLETE");

    let mut allowed = export_req(ns, "checkout");
    allowed.allow_incomplete = true;
    let bundle = export(&mut c, allowed).await;
    let verified = attested_bundle::verify(&bundle).unwrap();
    let Completeness::Incomplete(reasons) = verified.completeness else {
        panic!("a capped export must be marked incomplete");
    };
    assert!(reasons.contains(&IncompleteReason::SnapshotTruncated));
    assert!(reasons.contains(&IncompleteReason::ValidationIncomplete));
    assert!(reasons.contains(&IncompleteReason::UnresolvedEntities));
    assert!(
        verified
            .unresolved
            .iter()
            .any(|u| u.cause == UnresolvedCause::Missing),
        "{:?}",
        verified.unresolved
    );

    let mut relabelled = bundle.clone();
    relabelled.completeness = Some(Completeness::Complete.to_proto());
    attested_bundle::seal(&mut relabelled).unwrap();
    assert!(matches!(
        attested_bundle::verify(&relabelled),
        Err(BundleError::FalseComplete(_))
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn historical_export_describes_the_state_right_after_the_changeset() {
    let endpoint = start_server(None).await;
    let mut c = client(&endpoint).await;
    let ns = "attested-history";
    put(&mut c, event(ns, "placed")).await;
    put(&mut c, event_model(ns, "checkout", "Before", &["placed"])).await;
    let then = newest_changeset(&mut c).await;
    put(&mut c, event_model(ns, "checkout", "After", &["placed"])).await;

    let mut req = export_req(ns, "checkout");
    req.as_of_changeset = then.clone();
    let historical = export(&mut c, req.clone()).await;
    attested_bundle::verify(&historical).unwrap();
    let state = historical.state.as_ref().unwrap();
    assert_eq!(state.mode(), pb::attested_state::Mode::Historical);
    assert_eq!(state.baseline_changeset_id, then);
    assert!(historical.manifests.contains("Before"));
    assert!(!historical.manifests.contains("After"));

    let current = export(&mut c, export_req(ns, "checkout")).await;
    assert!(current.manifests.contains("After"));
    assert_ne!(current.bundle_digest, historical.bundle_digest);

    let mut on_branch = tonic::Request::new(req);
    on_branch.metadata_mut().insert(
        "x-trogon-atlas-branch",
        MetadataValue::try_from("feature").unwrap(),
    );
    let refused = c.export_attested_model(on_branch).await.unwrap_err();
    assert_eq!(refused.code(), tonic::Code::InvalidArgument);
}

fn model_entry(bundle: &pb::AttestedModelBundle) -> &pb::SnapshotEntry {
    bundle
        .entries
        .iter()
        .find(|e| e.entity.as_ref().unwrap().id.as_ref().unwrap().slug == "checkout")
        .unwrap()
}

fn reasons_of(bundle: &pb::AttestedModelBundle) -> Vec<IncompleteReason> {
    match attested_bundle::verify(bundle).unwrap().completeness {
        Completeness::Complete => Vec::new(),
        Completeness::Incomplete(reasons) => reasons,
    }
}

/// A process whose cached snapshot predates another writer's commit must
/// not attest the cached copy as current: it is carried, and marked.
#[tokio::test(flavor = "multi_thread")]
async fn a_model_changed_underneath_a_cached_snapshot_is_marked_not_mixed() {
    let store: Arc<dyn trogon_atlas_store::Store> =
        Arc::new(trogon_atlas_testsupport::shared().await.store().await);
    let reader =
        serve(EventModelServiceImpl::try_new(store.clone(), ServerMetrics::default()).unwrap())
            .await;
    let writer =
        serve(EventModelServiceImpl::try_new(store, ServerMetrics::default()).unwrap()).await;
    let mut r = client(&reader).await;
    let mut w = client(&writer).await;
    let ns = "attested-underneath";
    put(&mut w, event(ns, "placed")).await;
    put(&mut w, event_model(ns, "checkout", "Before", &["placed"])).await;
    let mut allowed = export_req(ns, "checkout");
    allowed.allow_incomplete = true;
    let before = export(&mut r, allowed.clone()).await;
    assert!(before.manifests.contains("Before"));

    put(&mut w, event_model(ns, "checkout", "After", &["placed"])).await;

    let refused = r
        .export_attested_model(export_req(ns, "checkout"))
        .await
        .unwrap_err();
    let failure = trogon_atlas_client::failure::Failure::from_status(&refused);
    assert_eq!(failure.code.unwrap().as_str(), "EXPORT_INCOMPLETE");
    assert!(
        failure.message.contains("REASON_CONCURRENT_WRITE"),
        "{refused:?}"
    );

    let marked = export(&mut r, allowed).await;
    let verified = attested_bundle::verify(&marked).unwrap();
    let reasons = reasons_of(&marked);
    assert!(reasons.contains(&IncompleteReason::ConcurrentWrite));
    assert!(reasons.contains(&IncompleteReason::UnresolvedEntities));
    let model_key = &model_entry(&marked).key;
    assert!(
        verified
            .unresolved
            .iter()
            .any(|u| &u.key == model_key && u.cause == UnresolvedCause::Changed),
        "{:?}",
        verified.unresolved
    );
    assert_eq!(
        model_entry(&marked).content_hash,
        model_entry(&before).content_hash,
        "the bundle carries the snapshot's copy, not the live one"
    );
    assert!(marked.manifests.contains("Before"));
    assert!(!marked.manifests.contains("After"));
}

/// Exports racing a writer never lock it out and never mix states: a
/// complete bundle is the revision its boundary names, and every bundle,
/// partial or not, agrees with itself across entries, manifests and
/// findings.
#[tokio::test(flavor = "multi_thread")]
async fn exports_racing_writes_never_mix_states() {
    let endpoint = start_server(None).await;
    let mut c = client(&endpoint).await;
    let ns = "attested-race";
    put(&mut c, event(ns, "placed")).await;
    put(&mut c, event_model(ns, "checkout", "w0", &["placed"])).await;
    let mut req = export_req(ns, "checkout");
    req.allow_incomplete = true;
    let mut bundles = vec![export(&mut c, req.clone()).await];

    const WRITES: usize = 31;
    let writer = {
        let mut c = client(&endpoint).await;
        tokio::spawn(async move {
            for i in 1..=WRITES {
                let members: &[&str] = if i % 2 == 0 { &["placed"] } else { &[] };
                put(
                    &mut c,
                    event_model(ns, "checkout", &format!("w{i}"), members),
                )
                .await;
            }
        })
    };
    while !writer.is_finished() {
        bundles.push(export(&mut c, req.clone()).await);
    }
    writer.await.unwrap();
    bundles.push(export(&mut c, req.clone()).await);

    let revisions = c
        .get_entity_history(pb::GetEntityHistoryRequest {
            r#ref: Some(pb::EntityRef {
                kind: pb::EntityKind::EventModel as i32,
                id: Some(id(ns, "checkout")),
            }),
            page_token: String::new(),
            page_size: 100,
        })
        .await
        .unwrap()
        .into_inner()
        .revisions;
    let mut saw_member = false;
    let mut saw_empty = false;
    for bundle in &bundles {
        let reasons = reasons_of(bundle);
        assert!(
            reasons.iter().all(|r| matches!(
                r,
                IncompleteReason::ConcurrentWrite | IncompleteReason::UnresolvedEntities
            )),
            "only the race may make a bundle partial: {reasons:?}"
        );
        let entry = model_entry(bundle);
        let landed_model = if reasons.is_empty() {
            let boundary = &bundle.state.as_ref().unwrap().baseline_changeset_id;
            let landed = revisions
                .iter()
                .filter(|r| r.changeset_id.as_str() <= boundary.as_str())
                .max_by(|a, b| a.changeset_id.cmp(&b.changeset_id))
                .expect("the boundary covers the model's first write");
            landed.after.as_ref().unwrap()
        } else {
            assert!(reasons.contains(&IncompleteReason::ConcurrentWrite));
            revisions
                .iter()
                .filter_map(|r| r.after.as_ref())
                .find(|after| {
                    trogon_atlas_core::entity_content_hash(after)
                        .unwrap()
                        .to_hex()
                        == entry.content_hash
                })
                .expect("a partial bundle still carries a revision that existed")
        };
        assert_eq!(
            entry.content_hash,
            trogon_atlas_core::entity_content_hash(landed_model)
                .unwrap()
                .to_hex(),
            "a complete bundle's model entry must be the revision its boundary names"
        );
        let Some(pb::entity::Kind::EventModel(em)) = landed_model.kind.as_ref() else {
            panic!("model revision is not an EventModel");
        };
        assert!(bundle.manifests.contains(&em.title));
        let has_member = !em.members.is_empty();
        saw_member |= has_member;
        saw_empty |= !has_member;
        assert_eq!(
            entry_slugs(bundle).contains(&"placed".to_string()),
            has_member
        );
        assert_eq!(
            mentions(&bundle.validation.as_ref().unwrap().issues, "placed"),
            has_member,
            "findings must come from the same state as the manifests"
        );
    }
    assert!(
        saw_member && saw_empty,
        "the race must observe both shapes of the model"
    );
}

fn signing_key_pem(seed: u8) -> String {
    use ed25519_dalek::pkcs8::{spki::der::pem::LineEnding, EncodePrivateKey};
    ed25519_dalek::SigningKey::from_bytes(&[seed; 32])
        .to_pkcs8_pem(LineEnding::LF)
        .unwrap()
        .to_string()
}

async fn start_signing_server(source: SigningKeySource, snapshot_cap: Option<usize>) -> String {
    let store = trogon_atlas_testsupport::shared().await.store().await;
    let mut svc = EventModelServiceImpl::try_new(Arc::new(store), ServerMetrics::default())
        .unwrap()
        .with_attested_signing_key(source);
    if let Some(cap) = snapshot_cap {
        svc = svc.with_snapshot_cap(cap);
    }
    serve(svc).await
}

#[tokio::test(flavor = "multi_thread")]
async fn signing_server_seals_bundles_its_public_key_authenticates() {
    let dir = tempfile::tempdir().unwrap();
    let key_file = dir.path().join("signing.pem");
    std::fs::write(&key_file, signing_key_pem(7)).unwrap();
    let signer = SigningKey::from_pkcs8_pem(&signing_key_pem(7)).unwrap();
    let endpoint = start_signing_server(SigningKeySource::File(key_file), None).await;
    let mut c = client(&endpoint).await;
    let ns = "attested-signed";
    put(&mut c, event(ns, "placed")).await;
    put(&mut c, event_model(ns, "checkout", "Checkout", &["placed"])).await;

    let bundle = export(&mut c, export_req(ns, "checkout")).await;

    assert_eq!(bundle.format, attested_bundle::SIGNED_BUNDLE_FORMAT);
    assert_eq!(
        attested_bundle::verify(&bundle).unwrap().authenticity,
        Authenticity::SignatureUnchecked(signer.key_id())
    );
    let trusted = TrustedKeys::new([signer.trusted_key()]).unwrap();
    let verified = attested_bundle::verify_trusted(&bundle, &trusted).unwrap();
    assert_eq!(
        verified.authenticity,
        Authenticity::Authenticated(signer.key_id())
    );
    assert!(verified.completeness.is_complete());

    let other = SigningKey::from_pkcs8_pem(&signing_key_pem(8)).unwrap();
    assert!(matches!(
        attested_bundle::verify_trusted(&bundle, &TrustedKeys::new([other.trusted_key()]).unwrap()),
        Err(BundleError::UntrustedKey(_))
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn signed_incomplete_export_stays_incomplete() {
    let dir = tempfile::tempdir().unwrap();
    let key_file = dir.path().join("signing.pem");
    std::fs::write(&key_file, signing_key_pem(9)).unwrap();
    let signer = SigningKey::from_pkcs8_pem(&signing_key_pem(9)).unwrap();
    let endpoint = start_signing_server(SigningKeySource::File(key_file), Some(1)).await;
    let mut c = client(&endpoint).await;
    let ns = "attested-signed-capped";
    put(&mut c, event(ns, "placed")).await;
    put(&mut c, event(ns, "shipped")).await;
    put(
        &mut c,
        event_model(ns, "checkout", "Checkout", &["placed", "shipped"]),
    )
    .await;

    let refused = c
        .export_attested_model(export_req(ns, "checkout"))
        .await
        .unwrap_err();
    let failure = trogon_atlas_client::failure::Failure::from_status(&refused);
    assert_eq!(failure.code.unwrap().as_str(), "EXPORT_INCOMPLETE");

    let mut allowed = export_req(ns, "checkout");
    allowed.allow_incomplete = true;
    let bundle = export(&mut c, allowed).await;
    let trusted = TrustedKeys::new([signer.trusted_key()]).unwrap();
    let verified = attested_bundle::verify_trusted(&bundle, &trusted).unwrap();
    assert_eq!(
        verified.authenticity,
        Authenticity::Authenticated(signer.key_id())
    );
    let Completeness::Incomplete(reasons) = verified.completeness else {
        panic!("signing must not turn a capped export complete");
    };
    assert!(reasons.contains(&IncompleteReason::SnapshotTruncated));
}

#[tokio::test(flavor = "multi_thread")]
async fn signing_key_that_cannot_be_loaded_fails_the_export_closed() {
    let dir = tempfile::tempdir().unwrap();
    let malformed = dir.path().join("malformed.pem");
    std::fs::write(&malformed, "not a key").unwrap();
    for (source, kind) in [
        (
            SigningKeySource::File(dir.path().join("absent.pem")),
            "file",
        ),
        (SigningKeySource::File(malformed), "file"),
        (
            SigningKeySource::EnvVar("TROGON_ATLAS_TEST_SIGNING_KEY_NEVER_SET".parse().unwrap()),
            "env",
        ),
    ] {
        let endpoint = start_signing_server(source, None).await;
        let mut c = client(&endpoint).await;
        let ns = "attested-signing-unavailable";
        put(&mut c, event(ns, "placed")).await;
        put(&mut c, event_model(ns, "checkout", "Checkout", &["placed"])).await;

        let mut req = export_req(ns, "checkout");
        req.allow_incomplete = true;
        let refused = c.export_attested_model(req).await.unwrap_err();
        assert_eq!(refused.code(), tonic::Code::FailedPrecondition);
        let failure = trogon_atlas_client::failure::Failure::from_status(&refused);
        assert_eq!(failure.code.unwrap().as_str(), "SIGNING_KEY_UNAVAILABLE");
        assert_eq!(
            failure.category,
            trogon_atlas_client::failure::FailureCategory::Precondition
        );
        let info = tonic_types::StatusExt::get_details_error_info(&refused).unwrap();
        assert_eq!(info.metadata.get("source").map(String::as_str), Some(kind));
        assert!(!refused.message().contains("absent.pem"), "{refused:?}");
    }
}

/// Races `writes` revisions of `slug` in `ns` on the baseline against
/// repeated exports sent by `request`, and returns every bundle.
async fn export_while_writing(
    endpoint: &str,
    ns: &'static str,
    slug: &'static str,
    request: impl Fn() -> tonic::Request<pb::ExportAttestedModelRequest>,
) -> Vec<pb::AttestedModelBundle> {
    let mut c = client(endpoint).await;
    let writer = {
        let mut c = client(endpoint).await;
        tokio::spawn(async move {
            for i in 1..=31 {
                put(
                    &mut c,
                    event_model(ns, slug, &format!("live{i}"), &["placed"]),
                )
                .await;
            }
        })
    };
    let mut bundles = Vec::new();
    while !writer.is_finished() {
        bundles.push(
            c.export_attested_model(request())
                .await
                .unwrap()
                .into_inner()
                .bundle
                .unwrap(),
        );
    }
    writer.await.unwrap();
    bundles
}

/// A historical export is rewound from history, so live writes after its
/// changeset cannot move it and must not mark it concurrent.
#[tokio::test(flavor = "multi_thread")]
async fn live_writes_do_not_taint_a_historical_export() {
    let endpoint = start_server(None).await;
    let mut c = client(&endpoint).await;
    let ns = "attested-history-live";
    put(&mut c, event(ns, "placed")).await;
    put(&mut c, event_model(ns, "checkout", "Before", &["placed"])).await;
    let then = newest_changeset(&mut c).await;

    let bundles = export_while_writing(&endpoint, ns, "checkout", || {
        let mut req = export_req(ns, "checkout");
        req.as_of_changeset = then.clone();
        tonic::Request::new(req)
    })
    .await;
    assert_ne!(
        bundles.len(),
        0,
        "the race must produce at least one export"
    );
    for bundle in &bundles {
        assert_eq!(reasons_of(bundle), Vec::<IncompleteReason>::new());
        assert!(bundle.manifests.contains("Before"));
    }
}

/// A baseline write to a key the branch overlays does not change what the
/// branch shows, so it must not mark the branch export concurrent.
#[tokio::test(flavor = "multi_thread")]
async fn baseline_writes_under_a_branch_overlay_do_not_taint_the_branch_export() {
    let endpoint = start_server(None).await;
    let mut c = client(&endpoint).await;
    let ns = "attested-overlay";
    let branch = "attested-overlay-feature";
    put(&mut c, event(ns, "placed")).await;
    put(&mut c, event_model(ns, "checkout", "Base", &["placed"])).await;
    c.create_branch(pb::CreateBranchRequest {
        name: branch.into(),
        doc: String::new(),
    })
    .await
    .unwrap();
    for entity in [
        event(ns, "placed"),
        event_model(ns, "checkout", "Branch", &["placed"]),
    ] {
        let mut req = tonic::Request::new(put_req(entity));
        req.metadata_mut().insert(
            "x-trogon-atlas-branch",
            MetadataValue::try_from(branch).unwrap(),
        );
        c.put_entity(req).await.unwrap();
    }

    let bundles = export_while_writing(&endpoint, ns, "checkout", || {
        let mut req = tonic::Request::new(export_req(ns, "checkout"));
        req.metadata_mut().insert(
            "x-trogon-atlas-branch",
            MetadataValue::try_from(branch).unwrap(),
        );
        req
    })
    .await;
    assert_ne!(
        bundles.len(),
        0,
        "the race must produce at least one export"
    );
    for bundle in &bundles {
        assert_eq!(reasons_of(bundle), Vec::<IncompleteReason>::new());
        assert!(bundle.manifests.contains("Branch"));
    }
}
