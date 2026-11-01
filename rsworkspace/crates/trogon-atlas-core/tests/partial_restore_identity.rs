#![allow(clippy::unwrap_used)]

use trogon_atlas_core::connection_persistence::{prepare_entity, prepare_landing};
use trogon_atlas_proto as pb;

#[test]
fn changing_title_does_not_allow_ordinary_landing_to_revive_a_removed_connection() {
    let base = prepare_entity(
        None,
        &pb::Entity {
            kind: Some(pb::entity::Kind::Ui(pb::Ui {
                id: Some(pb::Id {
                    namespace: "catalog".into(),
                    slug: "home".into(),
                    version: 1,
                }),
                title: "Home".into(),
                transitions: vec![pb::ui::Transition {
                    to: Some(pb::UiRef {
                        id: Some(pb::Id {
                            namespace: "catalog".into(),
                            slug: "cart".into(),
                            version: 1,
                        }),
                    }),
                    ..Default::default()
                }],
                ..Default::default()
            })),
            ..Default::default()
        },
    )
    .unwrap();
    let mut opposing = base.clone();
    let Some(pb::entity::Kind::Ui(body)) = opposing.kind.as_mut() else {
        panic!("expected UI")
    };
    body.transitions.clear();
    let opposing = prepare_entity(Some(&base), &opposing).unwrap();
    assert!(prepare_landing(Some(&base), Some(&opposing), &base).is_err());
    let mut selected = base.clone();
    let Some(pb::entity::Kind::Ui(body)) = selected.kind.as_mut() else {
        panic!("expected UI")
    };
    body.title = "Renamed home".into();
    let error = prepare_landing(Some(&base), Some(&opposing), &selected).unwrap_err();
    assert!(error.to_string().contains("removed"), "{error}");
}
