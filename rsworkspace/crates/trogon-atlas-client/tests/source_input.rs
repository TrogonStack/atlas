#![allow(clippy::unwrap_used)]

use std::fs;

use trogon_atlas_client::source_input::{
    read_bounded_file, read_supplied_bundle, SuppliedFilePath,
};

#[test]
fn explicit_supplied_files_retain_bytes_without_following_inventory_pointers() {
    let directory = tempfile::tempdir().unwrap();
    let inventory = directory.path().join("inventory.json");
    let payload = directory.path().join("payload");
    fs::write(&inventory, br#"{"profile":"supplied-native-v1","profile_revision":1,"registration":"reg","source_version":"opaque","partition":{"categories":["model"],"logical_roots":["native"],"selected_keys":[]},"coverage":"PARTIAL","resources":[{"key":{"category":"model","key":"a"},"file":"native/model.yaml","pointer":{"kind":"opaque","raw":"https://unavailable.invalid/private"}}],"relations":[]}"#).unwrap();
    fs::write(&payload, b"\0exact\xff\n").unwrap();
    let files = [
        SuppliedFilePath::new("native/model.yaml", "application/octet-stream", payload).unwrap(),
    ];
    let bundle = read_supplied_bundle(&inventory, &files).unwrap();
    assert_eq!(bundle.files()[0].bytes(), b"\0exact\xff\n");
    assert_eq!(bundle.inventory_bytes(), fs::read(inventory).unwrap());
}

#[test]
fn file_reads_are_bounded_and_errors_do_not_disclose_host_paths() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("private-file");
    fs::write(&path, b"12345").unwrap();
    assert_eq!(read_bounded_file(&path, 5).unwrap(), b"12345");
    let error = read_bounded_file(&path, 4).unwrap_err().to_string();
    assert!(!error.contains("private-file"));
    assert!(!error.contains(&directory.path().display().to_string()));
    assert!(read_bounded_file(directory.path(), 10).is_err());
    fs::remove_file(&path).unwrap();
    assert!(!read_bounded_file(&path, 10)
        .unwrap_err()
        .to_string()
        .contains("private-file"));
}

#[test]
fn file_metadata_is_explicit_and_logical_locators_are_validated_before_reads() {
    for name in [
        "../private",
        "/private",
        "https://host/file",
        "native//file",
    ] {
        assert!(SuppliedFilePath::new(name, "text/plain", "unused".into()).is_err());
    }
    assert!(SuppliedFilePath::new("native/file", "", "unused".into()).is_err());
}

#[cfg(unix)]
#[test]
fn explicitly_named_symlink_is_not_a_supplied_regular_file() {
    let directory = tempfile::tempdir().unwrap();
    let payload = directory.path().join("payload");
    fs::write(&payload, b"bytes").unwrap();
    let link = directory.path().join("link");
    std::os::unix::fs::symlink(payload, &link).unwrap();
    assert!(read_bounded_file(&link, 10).is_err());
}
