// Agents read these docs through list_entity_kinds and list_validation_rules
// and have no way to look up a numbered decision, so each doc must state
// its rule in full.

use trogon_atlas_server::validation::{entity_kind_catalog, rule_catalog};

#[test]
fn catalog_docs_do_not_cite_decision_numbers() {
    let mut offenders: Vec<String> = entity_kind_catalog()
        .into_iter()
        .filter(|k| k.doc.contains("Decision #"))
        .map(|k| k.proto_name)
        .collect();
    offenders.extend(
        rule_catalog()
            .into_iter()
            .filter(|r| r.doc.contains("Decision #"))
            .map(|r| r.code),
    );
    assert!(
        offenders.is_empty(),
        "catalog docs cite a decision number instead of stating the rule: {offenders:?}"
    );
}
