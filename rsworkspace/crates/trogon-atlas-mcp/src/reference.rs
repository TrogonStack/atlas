//! The generated MCP tool reference, rendered from the live tool registry and
//! `TOOL_POLICIES` so the published page can never describe a tool the
//! adapter does not serve.

use std::{fmt::Write as _, path::PathBuf};

use rmcp::model::Tool;

use crate::server::{McpServer, ToolPolicy, TOOL_POLICIES};

/// Where the committed reference lives, relative to this crate's manifest.
pub const REFERENCE_PATH: &str = "../../../docs/reference/mcp-tools.md";

pub const REGENERATE_COMMAND: &str = "mise run rust:mcp-reference";

pub fn reference_file() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(REFERENCE_PATH)
}

/// The registry and the policy table disagree, so no honest reference exists.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
#[error(
    "tool registry and TOOL_POLICIES disagree: tools without a policy row {tools_without_policy:?}; \
     policy rows without a tool {policies_without_tool:?}"
)]
pub struct RegistryMismatch {
    pub tools_without_policy: Vec<String>,
    pub policies_without_tool: Vec<String>,
}

/// The reference for the tools this crate registers.
pub fn generate() -> Result<String, RegistryMismatch> {
    render(&McpServer::registered_tools(), TOOL_POLICIES)
}

pub fn render(tools: &[Tool], policies: &[ToolPolicy]) -> Result<String, RegistryMismatch> {
    let mut rows: Vec<(&Tool, &ToolPolicy)> = Vec::with_capacity(tools.len());
    let mut tools_without_policy = Vec::new();
    for tool in tools {
        match policies.iter().find(|p| p.tool == tool.name) {
            Some(policy) => rows.push((tool, policy)),
            None => tools_without_policy.push(tool.name.to_string()),
        }
    }
    let mut policies_without_tool: Vec<String> = policies
        .iter()
        .filter(|p| !tools.iter().any(|t| t.name == p.tool))
        .map(|p| p.tool.to_owned())
        .collect();
    if !tools_without_policy.is_empty() || !policies_without_tool.is_empty() {
        tools_without_policy.sort();
        policies_without_tool.sort();
        return Err(RegistryMismatch {
            tools_without_policy,
            policies_without_tool,
        });
    }
    rows.sort_by(|a, b| a.0.name.cmp(&b.0.name));

    let mut out = String::new();
    out.push_str(HEADER);
    out.push_str(
        "| Tool | Minimum role | Effect | Idempotent | gRPC methods | Parameters | Description |\n\
         | --- | --- | --- | --- | --- | --- | --- |\n",
    );
    for (tool, policy) in rows {
        let _ = writeln!(
            out,
            "| `{}` | {} | {} | {} | {} | {} | {} |",
            tool.name,
            policy.min_role.as_str(),
            Effect::of(tool).as_str(),
            if is_idempotent(tool) { "yes" } else { "no" },
            code_list(policy.rpcs.iter().copied()),
            parameters(tool),
            cell(tool.description.as_deref().unwrap_or_default()),
        );
    }
    Ok(out)
}

const HEADER: &str = "# MCP tool reference

<!-- Generated from the trogon-atlas-mcp tool registry. Do not edit by hand; regenerate with `mise run rust:mcp-reference`. -->

Every tool the `/mcp` endpoint registers, rendered from the tool registry and
its role policy table. A crate test fails when this page is stale or when a
registered tool and its policy row disagree.

- **Minimum role** is the lowest role whose `tools/list` includes the tool.
  It is the highest role the server requires over the listed gRPC methods.
  The server still authorizes every call; the listing never grants access.
- When the server does not advertise the `who_am_i` feature, `tools/list`
  cannot learn the caller's role and lists every tool. When learning the role
  fails, it lists reader tools only.
- **Effect** comes from the tool's annotations: `read`, `write`, or
  `destructive` for writes annotated as able to remove or overwrite data.
- **Parameters** are the top-level input fields. A trailing `?` marks an
  optional field.

";

enum Effect {
    Read,
    Write,
    Destructive,
}

impl Effect {
    fn of(tool: &Tool) -> Self {
        let annotations = tool.annotations.as_ref();
        if annotations.and_then(|a| a.read_only_hint).unwrap_or(false) {
            Self::Read
        } else if annotations.is_some_and(rmcp::model::ToolAnnotations::is_destructive) {
            Self::Destructive
        } else {
            Self::Write
        }
    }

    fn as_str(&self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
            Self::Destructive => "destructive",
        }
    }
}

fn is_idempotent(tool: &Tool) -> bool {
    tool.annotations
        .as_ref()
        .and_then(|a| a.idempotent_hint)
        .unwrap_or(false)
}

fn parameters(tool: &Tool) -> String {
    let Some(properties) = tool
        .input_schema
        .get("properties")
        .and_then(serde_json::Value::as_object)
    else {
        return "none".to_owned();
    };
    if properties.is_empty() {
        return "none".to_owned();
    }
    let required: Vec<&str> = tool
        .input_schema
        .get("required")
        .and_then(serde_json::Value::as_array)
        .map(|names| names.iter().filter_map(serde_json::Value::as_str).collect())
        .unwrap_or_default();
    let mut names: Vec<String> = properties
        .keys()
        .map(|name| {
            if required.contains(&name.as_str()) {
                name.clone()
            } else {
                format!("{name}?")
            }
        })
        .collect();
    names.sort();
    code_list(names.iter().map(String::as_str))
}

fn code_list<'a>(items: impl Iterator<Item = &'a str>) -> String {
    items
        .map(|item| format!("`{item}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// One table cell: whitespace collapsed so a description stays on its row,
/// and pipes escaped so they do not split the cell.
fn cell(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace('|', "\\|")
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::sync::Arc;

    use rmcp::model::ToolAnnotations;

    use super::*;
    use crate::server::ToolRole;

    #[test]
    fn committed_reference_matches_the_registry() {
        let generated = generate().expect("registry and policies agree");
        let committed = std::fs::read_to_string(reference_file()).unwrap_or_default();
        assert!(
            generated == committed,
            "{REFERENCE_PATH} is stale; regenerate it with `{REGENERATE_COMMAND}`"
        );
    }

    #[test]
    fn generation_is_byte_identical_across_runs_and_registry_order() {
        let mut tools = McpServer::registered_tools();
        let first = render(&tools, TOOL_POLICIES).unwrap();
        tools.reverse();
        assert_eq!(first, render(&tools, TOOL_POLICIES).unwrap());
        assert_eq!(first, generate().unwrap());
    }

    fn tool(name: &'static str, annotations: ToolAnnotations) -> Tool {
        let schema = serde_json::json!({
            "type": "object",
            "properties": { "namespace": {}, "page_size": {} },
            "required": ["namespace"],
        });
        let serde_json::Value::Object(schema) = schema else {
            unreachable!()
        };
        Tool::new(name, "Reads | writes\n   things.", Arc::new(schema)).annotate(annotations)
    }

    const POLICIES: &[ToolPolicy] = &[
        ToolPolicy {
            tool: "alpha",
            rpcs: &["GetAlpha"],
            min_role: ToolRole::Reader,
        },
        ToolPolicy {
            tool: "beta",
            rpcs: &["PutBeta", "DeleteBeta"],
            min_role: ToolRole::Writer,
        },
    ];

    #[test]
    fn adding_a_tool_adds_its_row() {
        let alpha = tool("alpha", ToolAnnotations::new().read_only(true));
        let beta = tool(
            "beta",
            ToolAnnotations::new().read_only(false).destructive(true),
        );
        let without = render(std::slice::from_ref(&alpha), &POLICIES[..1]).unwrap();
        let with = render(&[beta, alpha], POLICIES).unwrap();

        assert!(!without.contains("`beta`"));
        assert!(with.contains(
            "| `alpha` | reader | read | no | `GetAlpha` | `namespace`, `page_size?` | Reads \\| writes things. |"
        ));
        assert!(with.contains("| `beta` | writer | destructive | no | `PutBeta`, `DeleteBeta` |"));
        assert!(with.find("`alpha`") < with.find("`beta`"));
    }

    #[test]
    fn a_tool_without_a_policy_row_refuses_to_render() {
        let alpha = tool("alpha", ToolAnnotations::new());
        let gamma = tool("gamma", ToolAnnotations::new());
        assert_eq!(
            render(&[alpha, gamma], &POLICIES[..1]),
            Err(RegistryMismatch {
                tools_without_policy: vec!["gamma".into()],
                policies_without_tool: vec![],
            })
        );
    }

    #[test]
    fn an_orphan_policy_row_refuses_to_render() {
        let alpha = tool("alpha", ToolAnnotations::new());
        assert_eq!(
            render(&[alpha], POLICIES),
            Err(RegistryMismatch {
                tools_without_policy: vec![],
                policies_without_tool: vec!["beta".into()],
            })
        );
    }
}
