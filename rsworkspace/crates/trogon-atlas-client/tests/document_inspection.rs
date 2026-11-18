#![allow(clippy::unwrap_used)]

use trogon_atlas_client::view::describe;
use trogon_atlas_core::transcode::TranscodePool;
use trogon_atlas_proto as pb;

#[test]
fn retained_document_inspection_preserves_filename_presence() {
    let types = TranscodePool::builtin().unwrap();
    let bodies = [
        pb::document::Body::Retained(pb::RetainedDocumentText {
            content: "retained text".into(),
            ..Default::default()
        }),
        pb::document::Body::RetainedContent(pb::RetainedContent {
            content: vec![0, 1, 255],
            ..Default::default()
        }),
    ];
    for body in bodies {
        for filename in [None, Some(String::new()), Some("report.txt".into())] {
            let entity = pb::Entity {
                kind: Some(pb::entity::Kind::Document(pb::Document {
                    id: Some(pb::Id {
                        namespace: "inspection".into(),
                        slug: "document".into(),
                        version: 1,
                    }),
                    body: Some(body.clone()),
                    filename: filename.clone().map(|value| pb::LogicalFilename { value }),
                    ..Default::default()
                })),
                ..Default::default()
            };
            let description = describe(&entity, &types, &[], &[]).unwrap();
            let json = serde_json::to_value(description).unwrap();
            let content = json["document_content"].as_object().unwrap();
            match filename {
                Some(value) => assert_eq!(content.get("filename"), Some(&serde_json::json!(value))),
                None => assert_eq!(content.get("filename"), None),
            }
        }
    }
}
