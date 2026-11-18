#![allow(clippy::unwrap_used)]
use serde_json::{json, Value};
use trogon_atlas_core::source_bundle::*;

fn inventory() -> Value {
    json!({"profile":"supplied-native-v1","profile_revision":1,"registration":"Registry-A",
        "source_version":"00.RC-a","partition":{"categories":["model"],"logical_roots":["native"],"selected_keys":[]},
        "resources":[{"key":{"category":"model","key":"order"},"file":"native/a.yaml","pointer":{"kind":"yaml","raw":"#/order"}}],"relations":[]})
}
fn file() -> SuppliedFile<'static> {
    SuppliedFile {
        logical_name: "native/a.yaml",
        media_type: "application/yaml",
        bytes: b"# original\n",
    }
}
fn validate(value: &Value) -> Result<ValidatedBundle, BundleError> {
    validate_bundle(&serde_json::to_vec(value).unwrap(), &[file()])
}
fn relation(id: &str) -> Value {
    json!({"owner":{"category":"model","key":"order"},"occurrence":id,"role":"observes","position":0,"target":{"kind":"external","raw":"https://unavailable.invalid/unknown"},"file":"native/a.yaml"})
}

#[test]
fn retains_opaque_identity_original_bytes_and_declared_coverage() {
    let mut bytes = serde_json::to_vec_pretty(&inventory()).unwrap();
    bytes.push(b'\n');
    let result = validate_bundle(&bytes, &[file()]).unwrap();
    assert_eq!(result.inventory_bytes(), bytes);
    assert_eq!(result.inventory().source_version().as_str(), "00.RC-a");
    assert_eq!(result.inventory().registration().as_str(), "Registry-A");
    assert_eq!(
        result.inventory().declared_coverage(),
        DeclaredCoverage::Partial
    );
    assert_eq!(result.files()[0].bytes(), file().bytes);
    assert_eq!(
        result.inventory_digest().purpose(),
        DigestPurpose::InventoryV1
    );
    assert_eq!(result.bundle_digest().purpose(), DigestPurpose::BundleV1);
    assert_eq!(
        result.files()[0].digest().purpose(),
        DigestPurpose::FileContentSha256
    );
    assert_eq!(result.files()[0].digest().hex().len(), 64);
}
#[test]
fn complete_empty_inventory_can_retain_partition_selected_keys() {
    let mut v = inventory();
    v["coverage"] = json!("COMPLETE");
    v["partition"]["selected_keys"] = json!([{"category":"model","key":"removed"}]);
    v["resources"] = json!([]);
    assert_eq!(
        validate(&v).unwrap().inventory().declared_coverage(),
        DeclaredCoverage::Complete
    );
}
#[test]
fn partition_scope_is_lexical_and_does_not_resolve_targets() {
    let mut v = inventory();
    v["relations"] = json!([relation("first"), relation("second")]);
    let result = validate(&v).unwrap();
    let relations = result.inventory().relations();
    assert_eq!(relations[0].target(), relations[1].target());
    assert_ne!(relations[0].occurrence(), relations[1].occurrence());
    assert_eq!(
        relations[0].target().raw().as_str(),
        "https://unavailable.invalid/unknown"
    );
    v["partition"]["logical_roots"] = json!(["nat"]);
    assert_eq!(validate(&v).unwrap_err(), BundleError::Partition);
    v = inventory();
    v["partition"]["categories"] = json!([]);
    assert!(validate(&v).is_err());
    v = inventory();
    v["partition"]["selected_keys"] = json!([{"category":"model","key":"other"}]);
    assert_eq!(validate(&v).unwrap_err(), BundleError::Partition);
}
#[test]
fn duplicate_durable_keys_and_unavailable_locators_reject() {
    let mut v = inventory();
    let r = v["resources"][0].clone();
    v["resources"].as_array_mut().unwrap().push(r);
    assert_eq!(validate(&v).unwrap_err(), BundleError::Duplicate);
    v = inventory();
    v["relations"] = json!([relation("same"), relation("same")]);
    assert_eq!(validate(&v).unwrap_err(), BundleError::Duplicate);
    v = inventory();
    v["relations"] = json!([relation("one")]);
    v["relations"][0]["owner"]["key"] = json!("missing");
    assert_eq!(validate(&v).unwrap_err(), BundleError::Unavailable);
    assert_eq!(
        validate_bundle(&serde_json::to_vec(&inventory()).unwrap(), &[]).unwrap_err(),
        BundleError::Unavailable
    );
    assert_eq!(
        validate_bundle(
            &serde_json::to_vec(&inventory()).unwrap(),
            &[file(), file()]
        )
        .unwrap_err(),
        BundleError::Duplicate
    );
}
#[test]
fn malformed_types_unknown_fields_duplicates_and_json_reject_without_disclosure() {
    for bytes in [
        b"\xff".as_slice(),
        b"{}{}",
        b"[",
        b"null",
        b"{\"secret\":1,\"secret\":2}",
        b"{\"x\":{\"a\":1,\"\\u0061\":2}}",
    ] {
        assert_eq!(
            validate_bundle(bytes, &[]).unwrap_err(),
            BundleError::Inventory
        );
    }
    for (field, value) in [
        ("profile_revision", json!("1")),
        ("profile_revision", json!(0)),
        ("profile", json!("other")),
        ("source_version", json!(1)),
        ("source_version", json!("")),
        ("registration", json!(" \n")),
        ("coverage", json!(null)),
        ("coverage", json!("complete")),
        ("resources", json!({})),
        ("extra-private-key", json!("private-value")),
    ] {
        let mut v = inventory();
        v[field] = value;
        let error = validate(&v).unwrap_err();
        assert!(!error.to_string().contains("private"));
    }
    let mut v = inventory();
    v["resources"][0]["pointer"]["extra"] = json!(1);
    assert!(validate(&v).is_err());
    let text = serde_json::to_string(&inventory()).unwrap().replace(
        "\"raw\":\"#/order\"",
        "\"raw\":\"#/order\",\"raw\":\"other\"",
    );
    assert!(validate_bundle(text.as_bytes(), &[file()]).is_err());
    let nested = format!(
        "{{\"unknown\":{}0{}}}",
        "[".repeat(MAX_JSON_DEPTH),
        "]".repeat(MAX_JSON_DEPTH)
    );
    assert!(validate_bundle(nested.as_bytes(), &[]).is_err());
}
#[test]
fn locators_and_media_types_are_validated_without_normalization() {
    for name in [
        "",
        "/a",
        "a/",
        "a//b",
        ".",
        "a/../b",
        "a/./b",
        "a\\b",
        "a\nb",
        "https://host/a",
        "C:/file",
        "a\u{7f}b",
    ] {
        assert!(LogicalName::new(name).is_err(), "{name:?}");
    }
    assert!(LogicalName::new(&"x".repeat(1024)).is_ok());
    assert!(LogicalName::new(&"x".repeat(1025)).is_err());
    assert_ne!(
        LogicalName::new("A/é").unwrap(),
        LogicalName::new("a/e\u{301}").unwrap()
    );
    for value in ["", "  ", "a\tb", "a\nb", "café"] {
        assert!(MediaType::new(value).is_err());
    }
    assert_eq!(
        MediaType::new(" text/plain ").unwrap().as_str(),
        " text/plain "
    );
    assert!(SourceVersion::new(&"x".repeat(1024)).is_ok());
    assert!(SourceVersion::new(&"x".repeat(1025)).is_err());
    assert!(RawPointer::new(&"x".repeat(16384)).is_ok());
    assert!(RawPointer::new(&"x".repeat(16385)).is_err());
}
#[test]
fn only_file_transport_order_is_digest_insensitive() {
    let bytes = serde_json::to_vec(&inventory()).unwrap();
    let extra = SuppliedFile {
        logical_name: "outside/z",
        media_type: "text/plain",
        bytes: b"opaque",
    };
    let a = validate_bundle(&bytes, &[file(), extra]).unwrap();
    let b = validate_bundle(&bytes, &[extra, file()]).unwrap();
    assert_eq!(a.bundle_digest(), b.bundle_digest());
    assert_eq!(a.files()[0].logical_name().as_str(), "native/a.yaml");
    let mut whitespace = bytes.clone();
    whitespace.push(b' ');
    assert_ne!(
        a.bundle_digest(),
        validate_bundle(&whitespace, &[file(), extra])
            .unwrap()
            .bundle_digest()
    );
    for (field, value) in [
        ("source_version", json!("00.RC-A")),
        ("coverage", json!("COMPLETE")),
    ] {
        let mut v = inventory();
        v[field] = value;
        assert_ne!(
            a.inventory_digest(),
            validate(&v).unwrap().inventory_digest()
        );
    }
    for changed in [
        SuppliedFile {
            bytes: b"changed",
            ..extra
        },
        SuppliedFile {
            media_type: "application/octet-stream",
            ..extra
        },
        SuppliedFile {
            logical_name: "outside/Z",
            ..extra
        },
    ] {
        assert_ne!(
            a.bundle_digest(),
            validate_bundle(&bytes, &[file(), changed])
                .unwrap()
                .bundle_digest()
        );
    }
    let mut v = inventory();
    v["relations"] = json!([relation("one"), relation("two")]);
    let a = validate(&v).unwrap();
    v["relations"].as_array_mut().unwrap().reverse();
    let b = validate(&v).unwrap();
    assert_ne!(a.bundle_digest(), b.bundle_digest());
    assert_eq!(b.inventory().relations()[0].occurrence().as_str(), "two");
}
#[test]
fn inventory_file_and_combined_byte_boundaries_are_inclusive() {
    let mut bytes = serde_json::to_vec(&inventory()).unwrap();
    bytes.resize(MAX_INVENTORY_BYTES, b' ');
    assert!(validate_bundle(&bytes, &[file()]).is_ok());
    bytes.push(b' ');
    assert_eq!(
        validate_bundle(&bytes, &[]).unwrap_err(),
        BundleError::Limit
    );
    let payload = vec![0; MAX_FILE_BYTES];
    let large = SuppliedFile {
        bytes: &payload,
        ..file()
    };
    assert!(validate_bundle(&serde_json::to_vec(&inventory()).unwrap(), &[large]).is_ok());
    let too_big = vec![0; MAX_FILE_BYTES + 1];
    assert_eq!(
        validate_bundle(
            b"",
            &[SuppliedFile {
                bytes: &too_big,
                ..file()
            }]
        )
        .unwrap_err(),
        BundleError::Limit
    );
    bytes.pop();
    let names: Vec<_> = (0..7).map(|i| format!("extra/{i}")).collect();
    let mut files = vec![large];
    files.extend(names.iter().map(|name| SuppliedFile {
        logical_name: name,
        media_type: "a",
        bytes: &payload,
    }));
    // Seven payloads plus a full inventory fit exactly; the eighth does not.
    assert!(validate_bundle(&bytes, &files[..7]).is_ok());
    assert_eq!(
        validate_bundle(&bytes, &files).unwrap_err(),
        BundleError::Limit
    );
}
#[test]
fn file_resource_and_relation_count_boundaries_are_inclusive() {
    let names: Vec<_> = (0..MAX_FILES - 1).map(|i| format!("extra/{i}")).collect();
    let mut files = vec![file()];
    files.extend(names.iter().map(|name| SuppliedFile {
        logical_name: name,
        media_type: "a",
        bytes: &[],
    }));
    let bytes = serde_json::to_vec(&inventory()).unwrap();
    assert!(validate_bundle(&bytes, &files).is_ok());
    files.push(file());
    assert_eq!(
        validate_bundle(&bytes, &files).unwrap_err(),
        BundleError::Limit
    );
    let mut v = inventory();
    let template = v["resources"][0].clone();
    v["resources"] = Value::Array(
        (0..MAX_RESOURCES)
            .map(|i| {
                let mut r = template.clone();
                r["key"]["key"] = json!(format!("r{i}"));
                r
            })
            .collect(),
    );
    assert!(validate(&v).is_ok());
    v["resources"].as_array_mut().unwrap().push(template);
    assert!(validate(&v).is_err());
    v = inventory();
    v["relations"] = Value::Array(
        (0..MAX_RELATIONS)
            .map(|i| relation(&format!("o{i}")))
            .collect(),
    );
    assert!(validate(&v).is_ok());
    v["relations"]
        .as_array_mut()
        .unwrap()
        .push(relation("extra"));
    assert!(validate(&v).is_err());
}

#[test]
fn json_depth_boundary_is_checked_before_strict_inventory_shape() {
    // No open extension field can admit nested arrays into a valid inventory.
    let at_limit = format!(
        "{{\"unknown\":{}0{}}}",
        "[".repeat(MAX_JSON_DEPTH - 1),
        "]".repeat(MAX_JSON_DEPTH - 1)
    );
    assert_eq!(
        validate_bundle(at_limit.as_bytes(), &[]).unwrap_err(),
        BundleError::Inventory
    );
    let beyond_limit = format!(
        "{{\"unknown\":{}0{}}}",
        "[".repeat(MAX_JSON_DEPTH),
        "]".repeat(MAX_JSON_DEPTH)
    );
    assert_eq!(
        validate_bundle(beyond_limit.as_bytes(), &[]).unwrap_err(),
        BundleError::JsonDepth
    );
}

#[test]
fn retained_bytes_are_independent_of_caller_buffers() {
    let mut inventory_bytes = serde_json::to_vec(&inventory()).unwrap();
    let mut file_bytes = b"immutable".to_vec();
    let result = validate_bundle(
        &inventory_bytes,
        &[SuppliedFile {
            bytes: &file_bytes,
            ..file()
        }],
    )
    .unwrap();
    inventory_bytes.fill(0);
    file_bytes.fill(0);
    assert_eq!(result.inventory().source_version().as_str(), "00.RC-a");
    assert_eq!(result.files()[0].bytes(), b"immutable");
    assert_eq!(
        result.inventory_bytes(),
        serde_json::to_vec(&inventory()).unwrap()
    );
}

fn positional_inventory(value: &Value) -> Value {
    json!([
        value["profile"],
        value["profile_revision"],
        value["registration"],
        value["source_version"],
        value["partition"],
        "PARTIAL",
        value["resources"],
        value["relations"]
    ])
}

#[test]
fn positional_records_are_rejected_at_every_inventory_record_position() {
    let mut cases = vec![positional_inventory(&inventory())];
    let mut v = inventory();
    v["partition"] = json!([["model"], ["native"], []]);
    cases.push(v);
    let mut v = inventory();
    v["resources"][0] = json!([{"category":"model","key":"order"}, "native/a.yaml", {"kind":"yaml","raw":"#/order"}]);
    cases.push(v);
    let mut v = inventory();
    v["resources"][0]["key"] = json!(["model", "order"]);
    cases.push(v);
    let mut v = inventory();
    v["resources"][0]["pointer"] = json!(["yaml", "#/order"]);
    cases.push(v);
    let mut v = inventory();
    v["partition"]["selected_keys"] = json!([["model", "order"]]);
    cases.push(v);
    let mut v = inventory();
    v["relations"] = json!([[{"category":"model","key":"order"}, "one", "observes", 0, {"kind":"external","raw":""}, "native/a.yaml"]]);
    cases.push(v);
    let mut v = inventory();
    v["relations"] = json!([relation("one")]);
    v["relations"][0]["owner"] = json!(["model", "order"]);
    cases.push(v);
    let mut v = inventory();
    v["relations"] = json!([relation("one")]);
    v["relations"][0]["target"] = json!(["external", "unresolved"]);
    cases.push(v);
    let accepted: Vec<_> = cases
        .iter()
        .enumerate()
        .filter_map(|(index, v)| validate(v).is_ok().then_some(index))
        .collect();
    assert!(
        accepted.is_empty(),
        "accepted positional record cases: {accepted:?}"
    );
}

#[test]
fn positional_inventory_cannot_bypass_resource_count_ceiling() {
    let mut v = inventory();
    let resource = v["resources"][0].clone();
    v["resources"] = Value::Array(
        (0..=MAX_RESOURCES)
            .map(|i| {
                let mut item = resource.clone();
                item["key"]["key"] = json!(format!("resource-{i}"));
                item
            })
            .collect(),
    );
    assert!(validate(&positional_inventory(&v)).is_err());
}

#[test]
fn positional_partition_cannot_bypass_partition_list_ceilings() {
    let categories: Vec<_> = (0..=MAX_RESOURCES)
        .map(|i| format!("category-{i}"))
        .collect();
    let roots: Vec<_> = (0..=MAX_RESOURCES)
        .map(|i| format!("native/root-{i}"))
        .collect();
    let selected: Vec<_> = (0..=MAX_RESOURCES)
        .map(|i| json!({"category":"model","key":format!("key-{i}")}))
        .collect();
    let partitions = [
        json!([categories, ["native"], []]),
        json!([["model"], roots, []]),
        json!([["model"], ["native"], selected]),
    ];
    let accepted: Vec<_> = partitions
        .iter()
        .enumerate()
        .filter_map(|(index, p)| {
            let mut v = inventory();
            v["resources"] = json!([]);
            v["partition"] = p.clone();
            validate(&v).is_ok().then_some(index)
        })
        .collect();
    assert!(
        accepted.is_empty(),
        "accepted oversized positional partitions: {accepted:?}"
    );
}

#[test]
fn explicit_coverage_requires_string_spelling() {
    for (spelling, expected) in [
        ("COMPLETE", DeclaredCoverage::Complete),
        ("PARTIAL", DeclaredCoverage::Partial),
    ] {
        let mut v = inventory();
        v["coverage"] = json!(spelling);
        assert_eq!(
            validate(&v).unwrap().inventory().declared_coverage(),
            expected
        );
    }
    let accepted: Vec<_> = [json!({"COMPLETE":null}), json!({"PARTIAL":null})]
        .into_iter()
        .filter(|coverage| {
            let mut v = inventory();
            v["coverage"] = coverage.clone();
            validate(&v).is_ok()
        })
        .collect();
    assert!(
        accepted.is_empty(),
        "accepted object-form coverage: {accepted:?}"
    );
}
