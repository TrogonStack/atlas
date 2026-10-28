use prost::Message;
use trogon_atlas_core::agent_metadata;
use trogon_atlas_proto as pb;

fn packed(name: &str, value: impl Message) -> prost_types::Any {
    prost_types::Any {
        type_url: format!("type.googleapis.com/{name}"),
        value: value.encode_to_vec(),
    }
}

fn event_with_field_annotation(annotation: prost_types::Any) -> pb::Entity {
    pb::Entity {
        kind: Some(pb::entity::Kind::Event(pb::Event {
            schema: Some(packed(
                "trogonatlas.eventmodel.v1alpha1.Schema",
                pb::Schema {
                    fields: vec![pb::FieldSpec {
                        name: "value".into(),
                        metadata: vec![annotation],
                        ..Default::default()
                    }],
                    ..Default::default()
                },
            )),
            ..Default::default()
        })),
        ..Default::default()
    }
}

#[test]
fn native_metadata_scope_is_checked_inside_schema_any() {
    for annotation in [
        packed(agent_metadata::AGENT, pb::AgentAnnotation::default()),
        packed(agent_metadata::BADGES, pb::BadgesAnnotation::default()),
        packed(
            agent_metadata::GROUPS,
            pb::ResourceGroupsAnnotation::default(),
        ),
        packed(
            agent_metadata::LIFECYCLE,
            pb::LifecycleAnnotation {
                status: "draft".into(),
                ..Default::default()
            },
        ),
    ] {
        let control = pb::Entity {
            kind: Some(pb::entity::Kind::Component(pb::Component {
                metadata: vec![annotation.clone()],
                ..Default::default()
            })),
            ..Default::default()
        };
        assert!(
            agent_metadata::validate(&control).is_ok(),
            "{}",
            annotation.type_url
        );
        let nested = event_with_field_annotation(annotation.clone());
        assert!(
            agent_metadata::validate(&nested).is_err(),
            "nested annotation accepted: {}",
            annotation.type_url
        );
    }
}

#[test]
fn ordinary_note_inside_schema_keeps_its_existing_scope() {
    let nested = event_with_field_annotation(packed(
        "trogonatlas.annotation.v1alpha1.NoteAnnotation",
        pb::NoteAnnotation {
            text: "Field attribution".into(),
            ..Default::default()
        },
    ));
    assert!(agent_metadata::validate(&nested).is_ok());
}
