//! Fixture decoding for the `seed` subcommand. Accepts prost-encoded
//! `.binpb` payloads and protobuf text format (`.textproto`/`.txtpb`),
//! transcoding the latter via the embedded file descriptor set.

use std::path::Path;

use anyhow::{Context as _, Result};
use prost::Message as _;
use prost_reflect::DynamicMessage;
use trogon_atlas_proto as pb;

const BATCH_MUTATE_REQUEST: &str = "trogonatlas.api.eventmodel.v1alpha1.BatchMutateRequest";

pub fn decode_batch_mutate_fixture(path: &Path, bytes: &[u8]) -> Result<pb::BatchMutateRequest> {
    let is_text = matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("textproto" | "txtpb" | "textpb")
    );
    if is_text {
        decode_textproto(bytes)
    } else {
        pb::BatchMutateRequest::decode(bytes)
            .with_context(|| format!("decoding {} as BatchMutateRequest", path.display()))
    }
}

fn decode_textproto(bytes: &[u8]) -> Result<pb::BatchMutateRequest> {
    let text = std::str::from_utf8(bytes).context("textproto fixture is not valid UTF-8")?;
    let desc = trogon_atlas_core::transcode::message_descriptor(BATCH_MUTATE_REQUEST)?;
    let dynamic =
        DynamicMessage::parse_text_format(desc, text).context("parsing textproto fixture")?;
    let buf = dynamic.encode_to_vec();
    pb::BatchMutateRequest::decode(buf.as_slice())
        .context("re-decoding transcoded BatchMutateRequest")
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn decodes_textproto_fixture() {
        let text = r#"
# comment lines are allowed
ops {
  put {
    entity {
      event {
        id { namespace: "shop" slug: "order-placed" version: 1 }
        title: "Order placed"
      }
    }
  }
}
"#;
        let req =
            decode_batch_mutate_fixture(Path::new("fixture.textproto"), text.as_bytes()).unwrap();
        assert_eq!(req.ops.len(), 1);
    }

    #[test]
    fn decodes_binpb_fixture() {
        let req = pb::BatchMutateRequest {
            ops: vec![pb::BatchMutateOp { op: None }],
            validate_only: true,
            operation_id: String::new(),
        };
        let bytes = req.encode_to_vec();
        let decoded = decode_batch_mutate_fixture(Path::new("fixture.binpb"), &bytes).unwrap();
        assert!(decoded.validate_only);
        assert_eq!(decoded.ops.len(), 1);
    }

    #[test]
    fn rejects_garbage_textproto() {
        let err = decode_batch_mutate_fixture(Path::new("x.textproto"), b"not { valid").is_err();
        assert!(err);
    }

    #[test]
    fn shipped_ecommerce_fixture_parses() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/ecommerce.textproto");
        let bytes =
            std::fs::read(&path).expect("testdata/ecommerce.textproto ships with the crate");
        let req = decode_batch_mutate_fixture(&path, &bytes).unwrap();
        assert_ne!(req.ops, [] as [trogon_atlas_proto::BatchMutateOp; 0]);
    }
}
