//! `import-git` writes land in the changeset log, so an imported entity has
//! history and the import can be reverted like any other write.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use tonic::Request;
use trogon_atlas_proto as pb;
use trogon_atlas_proto::event_model_service_server::EventModelService;
use trogon_atlas_server::{
    git_mirror::{import_entities, IMPORT_AUTHOR, IMPORT_RPC},
    metrics::ServerMetrics,
    service::EventModelServiceImpl,
};

const NS: &str = "git-import-history";

fn id(slug: &str) -> pb::Id {
    pb::Id {
        namespace: NS.into(),
        slug: slug.into(),
        version: 1,
    }
}

fn event(slug: &str) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::Event(pb::Event {
            id: Some(id(slug)),
            title: slug.into(),
            ..Default::default()
        })),
    }
}

fn event_ref(slug: &str) -> pb::EntityRef {
    pb::EntityRef {
        kind: pb::EntityKind::Event as i32,
        id: Some(id(slug)),
    }
}

async fn exists(svc: &EventModelServiceImpl, slug: &str) -> bool {
    match svc
        .get_entity(Request::new(pb::GetEntityRequest {
            kind: pb::EntityKind::Event as i32,
            id: Some(id(slug)),
        }))
        .await
    {
        Ok(_) => true,
        Err(status) if status.code() == tonic::Code::NotFound => false,
        Err(other) => panic!("unexpected get_entity error: {other}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn imported_entities_have_history_and_can_be_reverted() {
    let store: Arc<dyn trogon_atlas_store::Store> =
        Arc::new(trogon_atlas_testsupport::shared().await.store().await);
    let svc = EventModelServiceImpl::try_new(store.clone(), ServerMetrics::default()).unwrap();

    svc.put_entity(Request::new(pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(event("already-here")),
        create_only: true,
        if_match: String::new(),
        validate_only: false,
        force: false,
    }))
    .await
    .unwrap();

    let mirror = [
        event("order-placed"),
        event("already-here"),
        event("order-paid"),
    ];
    let report = import_entities(store.as_ref(), &mirror).await;
    assert_eq!((report.imported, report.skipped), (2, 1));
    assert_eq!(report.unrecorded, Vec::<String>::new());
    let [changeset_id] = report.changesets.as_slice() else {
        panic!("one changeset per chunk, got {:?}", report.changesets);
    };

    let changeset = svc
        .get_changeset(Request::new(pb::GetChangesetRequest {
            id: changeset_id.clone(),
        }))
        .await
        .unwrap()
        .into_inner()
        .changeset
        .unwrap();
    assert_eq!(changeset.rpc, IMPORT_RPC);
    assert_eq!(changeset.author, IMPORT_AUTHOR);
    assert_eq!(changeset.branch, "");
    let touched: Vec<_> = changeset.ops.iter().map(|op| op.entity.clone()).collect();
    assert_eq!(
        touched,
        vec![
            Some(event_ref("order-placed")),
            Some(event_ref("order-paid"))
        ],
        "an entity the import skipped is not attributed to it"
    );

    let revisions = svc
        .get_entity_history(Request::new(pb::GetEntityHistoryRequest {
            r#ref: Some(event_ref("order-placed")),
            page_token: String::new(),
            page_size: 10,
        }))
        .await
        .unwrap()
        .into_inner()
        .revisions;
    assert_eq!(revisions.len(), 1);
    assert_eq!(&revisions[0].changeset_id, changeset_id);
    assert_eq!(revisions[0].before, None);
    assert!(revisions[0].after.is_some());

    let retry = import_entities(store.as_ref(), &mirror).await;
    assert_eq!((retry.imported, retry.skipped), (0, 3));
    assert_eq!(
        retry.changesets,
        Vec::<String>::new(),
        "a retry that lands nothing records nothing"
    );

    svc.revert_changeset(Request::new(pb::RevertChangesetRequest {
        id: changeset_id.clone(),
        dry_run: false,
        operation_id: String::new(),
    }))
    .await
    .unwrap();
    assert!(!exists(&svc, "order-placed").await);
    assert!(!exists(&svc, "order-paid").await);
    assert!(
        exists(&svc, "already-here").await,
        "the revert only undoes the import"
    );
}
