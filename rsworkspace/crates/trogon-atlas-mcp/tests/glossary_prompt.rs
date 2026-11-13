#![allow(clippy::expect_used)]

use trogon_atlas_mcp::prompts::TaskPrompt;

#[test]
fn generation_task_is_discoverable_with_evidence_review_and_refresh_guidance() {
    let prompt = TaskPrompt::find("generate-glossary")
        .expect("glossary generation must have a discoverable task procedure");
    for tool in [
        "who_am_i",
        "inspect_glossary_sources",
        "put_entity_json",
        "get_glossary_term",
        "review_glossary_term",
        "list_glossary_entries",
        "compare_glossaries",
        "check_glossary_usage",
        "inspect_glossary_refresh",
    ] {
        assert!(prompt.tools.contains(&tool), "missing {tool}");
    }
    for boundary in [
        "exact supporting passage",
        "User principal",
        "Agent principal",
        "server-owned review",
        "new Term version",
        "run receipt",
    ] {
        assert!(prompt.body.contains(boundary), "lost boundary: {boundary}");
    }
    assert!(!prompt.result().messages.is_empty());
}
