#![allow(clippy::unwrap_used, clippy::expect_used)]
use trogon_atlas_proto as pb;
use trogon_atlas_server::conv::validate_entity_content;
fn id(slug: &str) -> pb::Id {
    pb::Id {
        namespace: "hosting-admission".into(),
        slug: slug.into(),
        version: 1,
    }
}
fn owner() -> pb::Entity {
    pb::Entity {
        kind: Some(pb::entity::Kind::Component(pb::Component {
            id: Some(id("portal")),
            title: "Portal".into(),
            ui_hostings: vec![pb::UiHosting {
                ui: Some(pb::UiRef {
                    id: Some(id("review")),
                }),
                label: Some(pb::CatalogTitle {
                    value: "Review".into(),
                }),
                ..Default::default()
            }],
            ..Default::default()
        })),
        ..Default::default()
    }
}
#[test]
fn mutation_boundary_rejects_invalid_hosting_values() {
    assert!(validate_entity_content(&owner()).is_ok());
    let mut accepted = Vec::new();
    for (case, name) in [
        "missing label",
        "missing UI",
        "zero UI version",
        "oversized doc",
        "hosting count",
        "empty identity",
    ]
    .into_iter()
    .enumerate()
    {
        let mut entity = owner();
        let Some(pb::entity::Kind::Component(value)) = &mut entity.kind else {
            unreachable!()
        };
        match case {
            0 => value.ui_hostings[0].label = None,
            1 => value.ui_hostings[0].ui = None,
            2 => {
                value.ui_hostings[0]
                    .ui
                    .as_mut()
                    .unwrap()
                    .id
                    .as_mut()
                    .unwrap()
                    .version = 0
            }
            3 => {
                value.ui_hostings[0].doc = Some(pb::CatalogText {
                    value: "x".repeat(16_385),
                })
            }
            4 => value
                .ui_hostings
                .resize(1_025, value.ui_hostings[0].clone()),
            _ => {
                value.ui_hostings[0].connection_id = Some(pb::ConnectionId {
                    value: String::new(),
                })
            }
        }
        match validate_entity_content(&entity) {
            Ok(()) => accepted.push(name),
            Err(error) => assert_eq!(error.code(), tonic::Code::InvalidArgument),
        }
    }
    assert!(
        accepted.is_empty(),
        "invalid hosting cases accepted: {accepted:?}"
    );
}
#[test]
fn empty_hostings_already_obey_the_architectural_owner_size_limit() {
    let mut entity = owner();
    let Some(pb::entity::Kind::Component(value)) = &mut entity.kind else {
        unreachable!()
    };
    value.ui_hostings.clear();
    value.doc = "x".repeat(1_048_576);
    let error = validate_entity_content(&entity).unwrap_err();
    assert!(
        error
            .message()
            .contains("architectural owner exceeds encoded byte limit"),
        "{error}"
    );
}
