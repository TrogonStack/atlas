#![allow(clippy::unwrap_used)]

use trogon_atlas_core::source_bundle::{validate_bundle, DigestPurpose, SuppliedFile};

const INVENTORY: &[u8] = br#"{"profile":"supplied-native-v1","profile_revision":1,"registration":"registry-a","source_version":"00.RC-A","partition":{"categories":["model"],"logical_roots":["native"],"selected_keys":[]},"resources":[],"relations":[]}
"#;

#[test]
fn framed_digest_vectors_match_independent_sha256_values() {
    let files = [
        SuppliedFile {
            logical_name: "native/z.bin",
            media_type: "application/octet-stream",
            bytes: &[0, 255, 10],
        },
        SuppliedFile {
            logical_name: "native/a.txt",
            media_type: "text/plain",
            bytes: b"abc",
        },
    ];
    let bundle = validate_bundle(INVENTORY, &files).unwrap();
    assert_eq!(
        bundle.inventory_digest().hex(),
        "f66680661550bd526c4f78461f1b31aa950d06cec6416419e44f384c92f03d84"
    );
    assert_eq!(
        bundle.bundle_digest().hex(),
        "0c537e98e184e879a8e2969132eea2c0896dc3d9a52711a9ae5ccb868da22cb9"
    );
    assert_eq!(
        bundle.files()[0].digest().hex(),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(
        bundle.files()[1].digest().hex(),
        "712450d3c4a79eea9509e75dc1dacdeff58034df538536cfae2da882bd8a0c50"
    );
    assert_eq!(
        bundle.inventory_digest().purpose(),
        DigestPurpose::InventoryV1
    );
    assert_eq!(bundle.bundle_digest().purpose(), DigestPurpose::BundleV1);
    assert_eq!(
        bundle.files()[0].digest().purpose(),
        DigestPurpose::FileContentSha256
    );
    let reversed = [files[1], files[0]];
    assert_eq!(
        validate_bundle(INVENTORY, &reversed)
            .unwrap()
            .bundle_digest(),
        bundle.bundle_digest()
    );
}
