#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::Duration;

use prost::Message;
use trogon_atlas_types::{
    compile, compile_with_imports, CompileError, CompileLimits, CompileRunner, Diagnostic,
    ProtoPackagePrefix, ProtoPath, SourceBundle, SourceFile, TypeLibrarySource,
};

fn library(prefix: &str, files: &[(&str, &str)]) -> TypeLibrarySource {
    TypeLibrarySource::new(
        ProtoPackagePrefix::parse(prefix).unwrap(),
        SourceBundle::new(
            files
                .iter()
                .map(|(p, c)| SourceFile::new(ProtoPath::parse(p).unwrap(), *c)),
            &CompileLimits::default(),
        )
        .unwrap(),
    )
}

fn diagnostics(err: CompileError) -> Vec<Diagnostic> {
    match err {
        CompileError::Diagnostics(d) => d,
        other => panic!("expected diagnostics, got {other:?}"),
    }
}

const ORDERS: &str = r#"syntax = "proto3";
package acme.orders.v1;
import "google/protobuf/timestamp.proto";
import "acme/orders/v1/line.proto";
message OrderPlaced {
  string order_id = 1;
  google.protobuf.Timestamp placed_at = 2;
  repeated Line lines = 3;
  message Audit { string by = 1; }
  map<string, string> labels = 4;
}
"#;

const LINE: &str = r#"syntax = "proto3";
package acme.orders.v1;
message Line { string sku = 1; int64 quantity = 2; }
"#;

#[test]
fn compiles_source_text_with_well_known_and_local_imports() {
    let lib = library(
        "acme.orders",
        &[
            ("acme/orders/v1/orders.proto", ORDERS),
            ("acme/orders/v1/line.proto", LINE),
        ],
    );
    let compiled = compile(&lib, &[]).unwrap();
    let names: Vec<&str> = compiled
        .file_descriptor_set()
        .file
        .iter()
        .map(prost_types::FileDescriptorProto::name)
        .collect();
    assert_eq!(
        names,
        vec!["acme/orders/v1/line.proto", "acme/orders/v1/orders.proto"]
    );
    let mut messages = compiled.message_names();
    messages.sort();
    assert_eq!(
        messages,
        vec![
            "acme.orders.v1.Line",
            "acme.orders.v1.OrderPlaced",
            "acme.orders.v1.OrderPlaced.Audit",
        ]
    );
    assert_eq!(compiled.digest(), lib.digest());
}

#[test]
fn compiling_the_same_source_is_byte_for_byte_deterministic() {
    let a = library(
        "acme.orders",
        &[
            ("acme/orders/v1/orders.proto", ORDERS),
            ("acme/orders/v1/line.proto", LINE),
        ],
    );
    let b = library(
        "acme.orders",
        &[
            ("acme/orders/v1/line.proto", LINE),
            ("acme/orders/v1/orders.proto", ORDERS),
        ],
    );
    let first = compile(&a, &[])
        .unwrap()
        .file_descriptor_set()
        .encode_to_vec();
    let second = compile(&b, &[])
        .unwrap()
        .file_descriptor_set()
        .encode_to_vec();
    assert_eq!(first, second);
    assert_eq!(a.digest(), b.digest());
}

#[test]
fn imports_resolve_against_dependency_libraries() {
    let money = library(
        "acme.money",
        &[(
            "acme/money/v1/money.proto",
            "syntax = \"proto3\";\npackage acme.money.v1;\nmessage Money { int64 cents = 1; }\n",
        )],
    );
    let billing = library(
        "acme.billing",
        &[(
            "acme/billing/v1/invoice.proto",
            "syntax = \"proto3\";\npackage acme.billing.v1;\nimport \"acme/money/v1/money.proto\";\nmessage Invoice { acme.money.v1.Money total = 1; }\n",
        )],
    );
    let compiled = compile(&billing, std::slice::from_ref(&money)).unwrap();
    assert_eq!(compiled.file_descriptor_set().file.len(), 1);

    let err = compile(&billing, &[]).unwrap_err();
    let d = diagnostics(err);
    assert_eq!(d[0].path, "acme/billing/v1/invoice.proto");
    assert_eq!((d[0].line, d[0].column), (3, 1));
    assert!(d[0].message.contains("acme/money/v1/money.proto"), "{d:?}");
}

#[test]
fn imports_resolve_against_files_compiled_earlier() {
    let money = library(
        "acme.money",
        &[(
            "acme/money/v1/money.proto",
            "syntax = \"proto3\";\npackage acme.money.v1;\nmessage Money { int64 cents = 1; }\n",
        )],
    );
    let billing = library(
        "acme.billing",
        &[(
            "acme/billing/v1/invoice.proto",
            "syntax = \"proto3\";\npackage acme.billing.v1;\nimport \"acme/money/v1/money.proto\";\nmessage Invoice { acme.money.v1.Money total = 1; }\n",
        )],
    );
    let compiled_money = compile(&money, &[]).unwrap();
    let compiled =
        compile_with_imports(&billing, &[], compiled_money.file_descriptor_set()).unwrap();
    assert_eq!(compiled.message_names(), vec!["acme.billing.v1.Invoice"]);
}

#[test]
fn syntax_errors_carry_path_line_and_column() {
    let lib = library(
        "acme",
        &[(
            "acme/broken.proto",
            "syntax = \"proto3\";\npackage acme;\nmessage X {\n  strin a = 1;\n}\n",
        )],
    );
    let d = diagnostics(compile(&lib, &[]).unwrap_err());
    assert_eq!(d.len(), 1);
    assert_eq!(d[0].path, "acme/broken.proto");
    assert_eq!((d[0].line, d[0].column), (4, 3));
    assert!(d[0].message.contains("strin"), "{d:?}");

    let parse = library(
        "acme",
        &[(
            "acme/parse.proto",
            "syntax = \"proto3\";\npackage acme;\nmessage {\n",
        )],
    );
    let d = diagnostics(compile(&parse, &[]).unwrap_err());
    assert_eq!(d[0].path, "acme/parse.proto");
    assert_eq!(d[0].line, 3);
}

#[test]
fn packages_outside_the_owned_prefix_are_rejected() {
    let lib = library(
        "acme.orders",
        &[
            (
                "acme/a.proto",
                "syntax = \"proto3\";\npackage acme.ordersx;\nmessage A {}\n",
            ),
            ("acme/b.proto", "syntax = \"proto3\";\nmessage B {}\n"),
            (
                "acme/c.proto",
                "syntax = \"proto3\";\npackage acme.orders.v1;\nmessage C {}\n",
            ),
        ],
    );
    let d = diagnostics(compile(&lib, &[]).unwrap_err());
    assert_eq!(d.len(), 2, "{d:?}");
    assert_eq!(d[0].path, "acme/a.proto");
    assert_eq!((d[0].line, d[0].column), (2, 1));
    assert!(d[0].message.contains("outside"), "{d:?}");
    assert_eq!(d[1].path, "acme/b.proto");
    assert!(d[1].message.contains("must declare a package"), "{d:?}");
}

#[test]
fn reserved_packages_and_atlas_imports_are_rejected() {
    assert!(ProtoPackagePrefix::parse("google.type").is_err());
    assert!(ProtoPackagePrefix::parse("trogonatlas.eventmodel").is_err());

    let lib = library(
        "acme",
        &[(
            "acme/x.proto",
            "syntax = \"proto3\";\npackage acme;\nimport \"trogonatlas/type/v1alpha1/id.proto\";\nmessage X {}\n",
        )],
    );
    let d = diagnostics(compile(&lib, &[]).unwrap_err());
    assert_eq!(d.len(), 1, "{d:?}");
    assert_eq!((d[0].line, d[0].column), (3, 1));
    assert!(d[0].message.contains("reserved"), "{d:?}");
}

#[test]
fn a_file_may_not_shadow_a_dependency_path() {
    let dep = library(
        "acme.money",
        &[(
            "acme/shared.proto",
            "syntax = \"proto3\";\npackage acme.money;\n",
        )],
    );
    let lib = library(
        "acme.billing",
        &[(
            "acme/shared.proto",
            "syntax = \"proto3\";\npackage acme.billing;\n",
        )],
    );
    let d = diagnostics(compile(&lib, std::slice::from_ref(&dep)).unwrap_err());
    assert!(d[0].message.contains("acme.money"), "{d:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn runner_compiles_off_the_executor() {
    let lib = library("acme.orders", &[("acme/orders/v1/line.proto", LINE)]);
    let compiled = CompileRunner::default()
        .compile(lib.clone(), Vec::new())
        .await
        .unwrap();
    assert_eq!(compiled.digest(), lib.digest());
}

#[tokio::test(flavor = "multi_thread")]
async fn runner_gives_up_when_no_compile_slot_frees_in_time() {
    let lib = library("acme.orders", &[("acme/orders/v1/line.proto", LINE)]);
    let runner = CompileRunner::new(0, Duration::from_millis(20));
    assert_eq!(
        runner.compile(lib, Vec::new()).await,
        Err(CompileError::TimedOut(Duration::from_millis(20)))
    );
}
