use serde_json::{json, Value};
use trogon_atlas_client::{client, source_integration as source};
use trogon_atlas_core::source_bundle::{validate_bundle, SuppliedFile};
use trogon_atlas_proto as pb;

pub async fn prepare(endpoint: &str, token: &str, namespace: String) -> Value {
    let mut admin = client::connect(endpoint, Some(token), 30).await.unwrap();
    let id = |slug: &str| pb::Id {
        namespace: namespace.clone(),
        slug: slug.into(),
        version: 1,
    };
    admin.put_entity(pb::PutEntityRequest {
        entity: Some(pb::Entity { kind: Some(pb::entity::Kind::TypeLibrary(pb::TypeLibrary {
            id: Some(id("acme.money")), title: "Money".into(),
            files: vec![pb::ProtoSourceFile {
                path: "acme/money/v1/money.proto".into(),
                content: "syntax = \"proto3\";\npackage acme.money.v1;\nmessage Money { int64 cents = 1; string currency = 2; map<string, string> labels = 3; }\n".into(),
            }], ..Default::default()
        })), ..Default::default() }), create_only: true, ..Default::default()
    }).await.unwrap();
    let key = pb::SourceResourceKey {
        registration_id: "surface-tenant-source".into(),
        category: "event".into(),
        key: "priced".into(),
    };
    let manifest = |cents: &str, labels: &str| {
        format!(
            "apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1\nkind: Event\nmetadata: {{namespace: {namespace}, name: priced}}\nspec:\n  title: Priced\n  doc: Locally retained rationale\n  schema:\n    '@type': type.googleapis.com/acme.money.v1.Money\n    cents: '{cents}'\n    currency: USD\n    labels: {labels}\n"
        )
    };
    let initial = source::native_payload(
        &mut admin,
        key.clone(),
        manifest("9007199254740993", "{z: last, a: first}").as_bytes(),
    )
    .await
    .unwrap();
    let original = admin
        .put_entity(pb::PutEntityRequest {
            entity: initial.entity,
            create_only: true,
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner();
    let observed = pb::SourceObservedOwner {
        owner: Some(pb::EntityRef {
            kind: pb::EntityKind::Event as i32,
            id: Some(id("priced")),
        }),
        uid: original
            .entity
            .as_ref()
            .unwrap()
            .system
            .as_ref()
            .unwrap()
            .uid
            .clone(),
        etag: original.etag,
    };
    let registration = source::create_source_registration(
        &mut admin,
        pb::CreateSourceRegistrationRequest {
            registration: Some(pb::SourceRegistration {
                id: "surface-tenant-source".into(),
                administration_namespace: namespace.clone(),
                destination_namespaces: vec![namespace.clone()],
                profile_revision: 1,
                ownership_policy_revision: "money-policy".into(),
                deletion_policy: pb::SourceDeletionPolicy::RetainNative as i32,
                ..Default::default()
            }),
        },
    )
    .await
    .unwrap();
    let registration_revision = registration["registration"]["revision"]
        .as_str()
        .unwrap()
        .to_owned();
    admin
        .create_source_mapping(pb::CreateSourceMappingRequest {
            registration_revision: registration_revision.clone(),
            mapping: Some(pb::SourceMapping {
                id: "surface-priced-schema".into(),
                source: Some(key.clone()),
                selection: Some(pb::SourceSelection {
                    selection: Some(pb::source_selection::Selection::Entity(
                        observed.owner.clone().unwrap(),
                    )),
                }),
                ownership: Some(pb::SourceFieldOwnership {
                    policy: Some(pb::source_field_ownership::Policy::Fields(
                        pb::SourceOwnedFields {
                            fields: vec![pb::SourceOwnedField::Schema as i32],
                        },
                    )),
                }),
                adoption: Some(pb::SourceAdoption {
                    observed: Some(observed.clone()),
                    authorized_adoption: true,
                }),
                ..Default::default()
            }),
        })
        .await
        .unwrap();
    let supplied = manifest("9007199254740994", "{z: last, a: first}");
    let incoming = source::native_payload(&mut admin, key.clone(), supplied.as_bytes())
        .await
        .unwrap();
    let mut scope = trogon_atlas_client::tenant_types::TypeScope::default();
    scope.add_namespace(namespace.clone());
    let types = trogon_atlas_client::tenant_types::transcode_pool(&mut admin, &scope)
        .await
        .unwrap();
    let inventory = serde_json::to_vec(&json!({
        "profile":"supplied-native-v1","profile_revision":1,"registration":"surface-tenant-source","source_version":"money-revision",
        "partition":{"categories":["event"],"logical_roots":["native"],"selected_keys":[]},"coverage":"PARTIAL",
        "resources":[{"key":{"category":"event","key":"priced"},"file":"native/event.yaml","pointer":{"kind":"native","raw":"#/priced"}}],"relations":[]
    })).unwrap();
    let bundle = validate_bundle(
        &inventory,
        &[SuppliedFile {
            logical_name: "native/event.yaml",
            media_type: "application/yaml",
            bytes: supplied.as_bytes(),
        }],
    )
    .unwrap();
    let snapshot = source::accept_source_snapshot(
        &mut admin,
        source::snapshot_request(
            "surface-tenant-source".into(),
            registration_revision.clone(),
            &bundle,
        ),
    )
    .await
    .unwrap();
    let snapshot_id = snapshot["snapshot"]["id"].as_str().unwrap().to_owned();

    types
        .message_json_value(
            "trogonatlas.api.eventmodel.v1alpha1.PreviewSourceRunRequest",
            &pb::PreviewSourceRunRequest {
                registration_id: "surface-tenant-source".into(),
                registration_revision,
                snapshot_id,
                run_id: "surface-tenant-run".into(),
                native_payloads: vec![incoming],
                ..Default::default()
            },
        )
        .unwrap()
}

pub fn assert_operations(value: &Value) {
    let schema = value["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find_map(|entry| entry.pointer("/operation/put/entity/event/schema"))
        .expect("typed native operation must be reported");
    assert_schema(schema);
}
pub fn assert_mapping(value: &Value) {
    assert_schema(&value["mapping"]["base"]["sourceValue"]["event"]["schema"]);
    assert_schema(&value["mapping"]["base"]["nativeValue"]["event"]["schema"]);
}
fn assert_schema(value: &Value) {
    assert_eq!(
        value,
        &json!({"@type":"type.googleapis.com/acme.money.v1.Money", "cents":"9007199254740994", "currency":"USD", "labels":{"a":"first","z":"last"}})
    );
}
