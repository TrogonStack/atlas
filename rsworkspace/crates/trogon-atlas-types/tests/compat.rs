#![allow(clippy::unwrap_used, clippy::expect_used)]

use trogon_atlas_types::{
    compile, BreakingChange, CompileLimits, CompiledLibrary, ProtoPackagePrefix, ProtoPath,
    SourceBundle, SourceFile, TypeLibrarySource,
};

fn compiled(body: &str) -> CompiledLibrary {
    compiled_in("acme.orders.v1", body)
}

fn compiled_in(package: &str, body: &str) -> CompiledLibrary {
    let source = format!("syntax = \"proto3\";\npackage {package};\n{body}\n");
    let lib = TypeLibrarySource::new(
        ProtoPackagePrefix::parse("acme").unwrap(),
        SourceBundle::new(
            [SourceFile::new(
                ProtoPath::parse("acme/orders.proto").unwrap(),
                source,
            )],
            &CompileLimits::default(),
        )
        .unwrap(),
    );
    compile(&lib, &[]).unwrap()
}

fn changes(before: &str, after: &str) -> Vec<BreakingChange> {
    compiled(after).breaking_changes_since(&compiled(before))
}

const BASE: &str = r"
message Order {
  string id = 1;
  int64 total = 2;
  repeated string tags = 3;
  oneof payer { string card = 4; string wallet = 5; }
}
enum Status { STATUS_UNSPECIFIED = 0; STATUS_OPEN = 1; STATUS_CLOSED = 2; }
";

struct Case {
    name: &'static str,
    after: &'static str,
    expect: fn(&[BreakingChange]) -> bool,
}

#[test]
fn compatible_evolutions_report_nothing() {
    let cases = [
        (
            "add a field",
            r"
message Order {
  string id = 1;
  int64 total = 2;
  repeated string tags = 3;
  oneof payer { string card = 4; string wallet = 5; }
  string note = 6;
}
enum Status { STATUS_UNSPECIFIED = 0; STATUS_OPEN = 1; STATUS_CLOSED = 2; }
",
        ),
        (
            "remove a field after reserving its number and name",
            r#"
message Order {
  reserved 2;
  reserved "total";
  string id = 1;
  repeated string tags = 3;
  oneof payer { string card = 4; string wallet = 5; }
}
enum Status { STATUS_UNSPECIFIED = 0; STATUS_OPEN = 1; STATUS_CLOSED = 2; }
"#,
        ),
        (
            "add an enum value and a message",
            r"
message Order {
  string id = 1;
  int64 total = 2;
  repeated string tags = 3;
  oneof payer { string card = 4; string wallet = 5; }
}
message Refund { string id = 1; }
enum Status { STATUS_UNSPECIFIED = 0; STATUS_OPEN = 1; STATUS_CLOSED = 2; STATUS_VOID = 3; }
",
        ),
        (
            "remove an enum value after reserving it",
            r#"
message Order {
  string id = 1;
  int64 total = 2;
  repeated string tags = 3;
  oneof payer { string card = 4; string wallet = 5; }
}
enum Status { reserved 2; reserved "STATUS_CLOSED"; STATUS_UNSPECIFIED = 0; STATUS_OPEN = 1; }
"#,
        ),
    ];
    for (name, after) in cases {
        assert_eq!(changes(BASE, after), Vec::new(), "{name}");
    }
}

#[test]
fn each_breaking_rule_is_detected() {
    let cases = [
        Case {
            name: "field removed without reserving",
            after: r"
message Order {
  string id = 1;
  repeated string tags = 3;
  oneof payer { string card = 4; string wallet = 5; }
}
enum Status { STATUS_UNSPECIFIED = 0; STATUS_OPEN = 1; STATUS_CLOSED = 2; }
",
            expect: |c| {
                c == [BreakingChange::FieldRemoved {
                    message: "acme.orders.v1.Order".into(),
                    number: 2,
                    name: "total".into(),
                }]
            },
        },
        Case {
            name: "field removed with only its number reserved",
            after: r"
message Order {
  reserved 2;
  string id = 1;
  repeated string tags = 3;
  oneof payer { string card = 4; string wallet = 5; }
}
enum Status { STATUS_UNSPECIFIED = 0; STATUS_OPEN = 1; STATUS_CLOSED = 2; }
",
            expect: |c| matches!(c, [BreakingChange::FieldRemoved { number: 2, .. }]),
        },
        Case {
            name: "number reused with another type",
            after: r"
message Order {
  string id = 1;
  string total = 2;
  repeated string tags = 3;
  oneof payer { string card = 4; string wallet = 5; }
}
enum Status { STATUS_UNSPECIFIED = 0; STATUS_OPEN = 1; STATUS_CLOSED = 2; }
",
            expect: |c| {
                c == [BreakingChange::FieldNumberReused {
                    message: "acme.orders.v1.Order".into(),
                    number: 2,
                    from: "int64 total".into(),
                    to: "string total".into(),
                }]
            },
        },
        Case {
            name: "number reused with another name",
            after: r"
message Order {
  string id = 1;
  int64 amount = 2;
  repeated string tags = 3;
  oneof payer { string card = 4; string wallet = 5; }
}
enum Status { STATUS_UNSPECIFIED = 0; STATUS_OPEN = 1; STATUS_CLOSED = 2; }
",
            expect: |c| matches!(c, [BreakingChange::FieldNumberReused { number: 2, .. }]),
        },
        Case {
            name: "cardinality changed",
            after: r"
message Order {
  string id = 1;
  int64 total = 2;
  string tags = 3;
  oneof payer { string card = 4; string wallet = 5; }
}
enum Status { STATUS_UNSPECIFIED = 0; STATUS_OPEN = 1; STATUS_CLOSED = 2; }
",
            expect: |c| matches!(c, [BreakingChange::FieldCardinalityChanged { field, .. }] if field == "tags"),
        },
        Case {
            name: "field moved out of its oneof",
            after: r"
message Order {
  string id = 1;
  int64 total = 2;
  repeated string tags = 3;
  string card = 4;
  oneof payer { string wallet = 5; }
}
enum Status { STATUS_UNSPECIFIED = 0; STATUS_OPEN = 1; STATUS_CLOSED = 2; }
",
            expect: |c| {
                c == [BreakingChange::FieldOneofChanged {
                    message: "acme.orders.v1.Order".into(),
                    field: "card".into(),
                    from: "payer".into(),
                    to: String::new(),
                }]
            },
        },
        Case {
            name: "field moved into a oneof",
            after: r"
message Order {
  int64 total = 2;
  repeated string tags = 3;
  oneof payer { string card = 4; string wallet = 5; string id = 1; }
}
enum Status { STATUS_UNSPECIFIED = 0; STATUS_OPEN = 1; STATUS_CLOSED = 2; }
",
            expect: |c| matches!(c, [BreakingChange::FieldOneofChanged { field, .. }] if field == "id"),
        },
        Case {
            name: "json_name changed",
            after: r#"
message Order {
  string id = 1 [json_name = "orderId"];
  int64 total = 2;
  repeated string tags = 3;
  oneof payer { string card = 4; string wallet = 5; }
}
enum Status { STATUS_UNSPECIFIED = 0; STATUS_OPEN = 1; STATUS_CLOSED = 2; }
"#,
            expect: |c| {
                c == [BreakingChange::FieldJsonNameChanged {
                    message: "acme.orders.v1.Order".into(),
                    field: "id".into(),
                    from: "id".into(),
                    to: "orderId".into(),
                }]
            },
        },
        Case {
            name: "enum value removed",
            after: r"
message Order {
  string id = 1;
  int64 total = 2;
  repeated string tags = 3;
  oneof payer { string card = 4; string wallet = 5; }
}
enum Status { STATUS_UNSPECIFIED = 0; STATUS_OPEN = 1; }
",
            expect: |c| {
                c == [BreakingChange::EnumValueRemoved {
                    name: "acme.orders.v1.Status".into(),
                    value: "STATUS_CLOSED".into(),
                    number: 2,
                }]
            },
        },
        Case {
            name: "enum value renumbered",
            after: r"
message Order {
  string id = 1;
  int64 total = 2;
  repeated string tags = 3;
  oneof payer { string card = 4; string wallet = 5; }
}
enum Status { STATUS_UNSPECIFIED = 0; STATUS_OPEN = 1; STATUS_CLOSED = 3; }
",
            expect: |c| {
                c == [BreakingChange::EnumValueRenumbered {
                    name: "acme.orders.v1.Status".into(),
                    value: "STATUS_CLOSED".into(),
                    from: 2,
                    to: 3,
                }]
            },
        },
        Case {
            name: "message removed",
            after: r"
enum Status { STATUS_UNSPECIFIED = 0; STATUS_OPEN = 1; STATUS_CLOSED = 2; }
",
            expect: |c| {
                c == [BreakingChange::MessageRemoved {
                    message: "acme.orders.v1.Order".into(),
                }]
            },
        },
    ];
    for case in cases {
        let found = changes(BASE, case.after);
        assert!((case.expect)(&found), "{}: {found:?}", case.name);
    }
}

#[test]
fn changing_the_package_is_breaking() {
    let before = compiled_in("acme.orders.v1", BASE);
    let after = compiled_in("acme.orders.v2", BASE);
    let found = after.breaking_changes_since(&before);
    assert!(found.contains(&BreakingChange::PackageChanged {
        file: "acme/orders.proto".into(),
        from: "acme.orders.v1".into(),
        to: "acme.orders.v2".into(),
    }));
    assert!(found.contains(&BreakingChange::MessageRemoved {
        message: "acme.orders.v1.Order".into(),
    }));
}

#[test]
fn breaking_changes_render_as_readable_reasons() {
    let found = changes(
        BASE,
        r"
message Order {
  string id = 1;
  repeated string tags = 3;
  oneof payer { string card = 4; string wallet = 5; }
}
enum Status { STATUS_UNSPECIFIED = 0; STATUS_OPEN = 1; STATUS_CLOSED = 2; }
",
    );
    assert_eq!(
        found[0].to_string(),
        "acme.orders.v1.Order: field total = 2 was removed without reserving its number and name"
    );
}
