#![allow(clippy::unwrap_used)]

use trogon_atlas_mcp::{
    profile::ToolProfile,
    prompts::TaskPrompt,
    server::{McpServer, ToolRole, TOOL_POLICIES},
};

#[test]
fn lean_glossary_readers_are_discoverable_in_equivalent_profiles_and_generation_prompt() {
    let tools = McpServer::registered_tools();
    let prompt = TaskPrompt::find("generate-glossary").unwrap();
    for (name, rpc) in [
        ("list_glossary_pointers", "ListGlossaryPointers"),
        ("get_glossary_definition", "GetGlossaryDefinition"),
        ("check_glossary_code_usage", "CheckGlossaryCodeUsage"),
    ] {
        let tool = tools.iter().find(|tool| tool.name == name).unwrap();
        let policy = TOOL_POLICIES
            .iter()
            .find(|policy| policy.tool == name)
            .unwrap();
        assert_eq!(policy.rpcs, &[rpc]);
        assert_eq!(policy.min_role, ToolRole::Reader);
        let annotations = tool.annotations.as_ref().unwrap();
        assert_eq!(annotations.read_only_hint, Some(true));
        assert_eq!(annotations.destructive_hint, Some(false));
        assert_eq!(annotations.idempotent_hint, Some(true));
        for profile in ToolProfile::ALL {
            assert!(profile.admits(tool), "{profile}: {name}");
        }
        assert!(prompt.tools.contains(&name));
    }
}
