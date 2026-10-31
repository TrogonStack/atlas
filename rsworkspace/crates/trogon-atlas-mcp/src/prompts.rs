//! Task prompts: the procedure for one task, fetched by the session doing
//! it, so the server instructions every session pays for stay short.

use rmcp::model::{GetPromptResult, Prompt, PromptMessage, Role};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TaskPrompt {
    pub name: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub body: &'static str,
    /// Every tool the body names. A session is offered the prompt only when
    /// it is offered all of them, so a prompt never directs a session to a
    /// tool it cannot call.
    pub tools: &'static [&'static str],
}

pub const TASK_PROMPTS: &[TaskPrompt] = &[
    TaskPrompt {
        name: "author-storyboard",
        title: "Author a storyboard",
        description: "The storyboard authoring rules validation enforces, and the follow-up after each mutation.",
        body: include_str!("../prompts/author-storyboard.md"),
        tools: &[
            "get_entity_json",
            "list_entities",
            "put_entity_json",
            "validate_event_model",
        ],
    },
    TaskPrompt {
        name: "design-strategic-layout",
        title: "Design the strategic layout",
        description: "Where bounded contexts, the problem space and their language live, and how contexts relate.",
        body: include_str!("../prompts/design-strategic-layout.md"),
        tools: &["put_entity_json", "validate_project"],
    },
    TaskPrompt {
        name: "author-specification",
        title: "Author a specification",
        description: "How to record contract artifacts in the native specification entity.",
        body: include_str!("../prompts/author-specification.md"),
        tools: &["get_entity_json", "get_entity_schema", "put_entity_json"],
    },
    TaskPrompt {
        name: "declare-reliability",
        title: "Declare reliability",
        description: "How service level indicators, objectives and alert policies point into the model.",
        body: include_str!("../prompts/declare-reliability.md"),
        tools: &["put_entity_json", "validate_event_model"],
    },
    TaskPrompt {
        name: "repair-validation-finding",
        title: "Repair a validation finding",
        description: "Scope a finding, gather its dependencies, apply a guarded repair and verify it.",
        body: include_str!("../prompts/repair-validation-finding.md"),
        tools: &[
            "batch_mutate_json",
            "delete_entity",
            "get_entity_json",
            "get_impact",
            "get_incoming_references",
            "get_outgoing_references",
            "list_validation_rules",
            "retarget_references",
            "validate_event_model",
            "validate_project",
        ],
    },
];

impl TaskPrompt {
    pub fn find(name: &str) -> Option<&'static TaskPrompt> {
        TASK_PROMPTS.iter().find(|prompt| prompt.name == name)
    }

    pub fn listing(&self) -> Prompt {
        Prompt::new(self.name, Some(self.description), None).with_title(self.title)
    }

    pub fn result(&self) -> GetPromptResult {
        GetPromptResult::new(vec![PromptMessage::new_text(Role::User, self.body)])
            .with_description(self.description)
    }
}

/// The registered tool names `text` mentions as whole words.
#[cfg(test)]
pub(crate) fn mentioned_tools(text: &str) -> std::collections::BTreeSet<String> {
    let registered: std::collections::BTreeSet<String> =
        crate::server::McpServer::registered_tools()
            .into_iter()
            .map(|tool| tool.name.to_string())
            .collect();
    text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|word| registered.contains(*word))
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use std::collections::BTreeSet;

    use super::*;
    use crate::{profile::ToolProfile, server::McpServer};

    fn tool(name: &str) -> Option<rmcp::model::Tool> {
        McpServer::registered_tools()
            .into_iter()
            .find(|t| t.name == name)
    }

    #[test]
    fn prompt_names_are_unique() {
        let names: BTreeSet<&str> = TASK_PROMPTS.iter().map(|p| p.name).collect();
        assert_eq!(names.len(), TASK_PROMPTS.len());
    }

    #[test]
    fn every_listed_tool_is_registered() {
        for prompt in TASK_PROMPTS {
            for name in prompt.tools {
                assert!(
                    tool(name).is_some(),
                    "{} lists unknown tool {name}",
                    prompt.name
                );
            }
        }
    }

    #[test]
    fn the_listed_tools_are_exactly_the_tools_the_body_names() {
        for prompt in TASK_PROMPTS {
            let listed: BTreeSet<String> = prompt.tools.iter().map(|t| (*t).to_owned()).collect();
            assert_eq!(mentioned_tools(prompt.body), listed, "{}", prompt.name);
        }
    }

    #[test]
    fn each_task_profile_offers_its_prompts() {
        let offers = |profile: ToolProfile, name: &str| {
            TaskPrompt::find(name)
                .unwrap()
                .tools
                .iter()
                .all(|t| profile.admits(&tool(t).unwrap()))
        };
        for name in [
            "author-storyboard",
            "design-strategic-layout",
            "author-specification",
            "declare-reliability",
        ] {
            assert!(
                offers(ToolProfile::Authoring, name),
                "authoring hides {name}"
            );
        }
        for prompt in TASK_PROMPTS {
            assert!(
                offers(ToolProfile::Repair, prompt.name),
                "repair hides {}",
                prompt.name
            );
        }
    }

    #[test]
    fn prompts_keep_the_strategic_layout_and_validation_followup() {
        let all: String = TASK_PROMPTS
            .iter()
            .flat_map(|p| p.body.split_whitespace())
            .collect::<Vec<_>>()
            .join(" ");
        for needle in [
            "ONE problem-space namespace",
            "DOMAIN_MISSING_PROJECT",
            "validate_project with the namespace or project scope",
            "`issues` array",
            "Errors and Warnings",
        ] {
            assert!(all.contains(needle), "doctrine lost: {needle}");
        }
    }

    #[test]
    fn a_prompt_result_carries_its_body() {
        let prompt = TaskPrompt::find("author-storyboard").unwrap();
        let result = prompt.result();
        assert_eq!(result.messages.len(), 1);
        assert_eq!(
            prompt.listing().title.as_deref(),
            Some("Author a storyboard")
        );
        assert!(TaskPrompt::find("everything").is_none());
    }
}
