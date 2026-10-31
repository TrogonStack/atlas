//! Tool profiles: named task scopes that narrow the tools an MCP session is
//! offered and may call. A profile is derived from each tool's registry
//! annotations and its policy row's area, never from a hand-kept tool list,
//! so a new tool joins the profiles its area and effect imply.

use std::{fmt, str::FromStr};

use rmcp::model::Tool;

use crate::server::{policy_for, ToolArea};

pub const PROFILE_HEADER: &str = "x-trogon-atlas-profile";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ToolProfile {
    Authoring,
    Repair,
    Review,
    SourceIntegration,
    ReadOnly,
}

impl ToolProfile {
    pub const ALL: &'static [ToolProfile] = &[
        ToolProfile::Authoring,
        ToolProfile::Repair,
        ToolProfile::Review,
        ToolProfile::SourceIntegration,
        ToolProfile::ReadOnly,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            ToolProfile::Authoring => "authoring",
            ToolProfile::Repair => "repair",
            ToolProfile::Review => "review",
            ToolProfile::SourceIntegration => "source-integration",
            ToolProfile::ReadOnly => "read-only",
        }
    }

    /// Whether this profile offers `tool`. A tool without a policy row is in
    /// no profile, so an incomplete registry hides a tool instead of
    /// exposing it.
    pub fn admits(self, tool: &Tool) -> bool {
        policy_for(&tool.name)
            .is_some_and(|policy| self.admits_shape(policy.area, ToolEffect::of(tool)))
    }

    fn admits_shape(self, area: ToolArea, effect: ToolEffect) -> bool {
        use ToolArea::{Administration, Branch, Discovery, Model, Source};
        let reads_the_model =
            effect == ToolEffect::Read && matches!(area, Discovery | Model | Branch);
        let authors =
            reads_the_model || (effect == ToolEffect::Write && matches!(area, Model | Branch));
        match self {
            ToolProfile::ReadOnly => effect == ToolEffect::Read && area != Administration,
            ToolProfile::Review => reads_the_model,
            ToolProfile::Authoring => authors,
            ToolProfile::Repair => {
                authors || (effect == ToolEffect::Destructive && matches!(area, Model | Branch))
            }
            ToolProfile::SourceIntegration => {
                area == Source || (effect == ToolEffect::Read && matches!(area, Discovery | Model))
            }
        }
    }
}

impl fmt::Display for ToolProfile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown tool profile `{given}`; expected one of: {}", ToolProfile::ALL.iter().map(|p| p.as_str()).collect::<Vec<_>>().join(", "))]
pub struct UnknownProfile {
    pub given: String,
}

impl FromStr for ToolProfile {
    type Err = UnknownProfile;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let given = s.trim();
        ToolProfile::ALL
            .iter()
            .copied()
            .find(|profile| profile.as_str() == given)
            .ok_or_else(|| UnknownProfile {
                given: given.to_owned(),
            })
    }
}

/// What a tool does to the model, read from its registry annotations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolEffect {
    Read,
    Write,
    Destructive,
}

impl ToolEffect {
    pub fn of(tool: &Tool) -> Self {
        let annotations = tool.annotations.as_ref();
        if annotations.and_then(|a| a.read_only_hint).unwrap_or(false) {
            Self::Read
        } else if annotations.is_some_and(rmcp::model::ToolAnnotations::is_destructive) {
            Self::Destructive
        } else {
            Self::Write
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
            Self::Destructive => "destructive",
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use crate::server::{McpServer, TOOL_POLICIES};

    fn tool(name: &str) -> Tool {
        McpServer::registered_tools()
            .into_iter()
            .find(|t| t.name == name)
            .unwrap()
    }

    #[test]
    fn every_profile_name_round_trips() {
        for &profile in ToolProfile::ALL {
            assert_eq!(profile.as_str().parse::<ToolProfile>(), Ok(profile));
        }
    }

    #[test]
    fn an_unknown_profile_names_the_known_ones() {
        let err = "everything".parse::<ToolProfile>().unwrap_err();
        assert_eq!(err.given, "everything");
        assert!(err.to_string().contains("source-integration"), "{err}");
    }

    #[test]
    fn every_tool_outside_administration_has_a_profile_home() {
        for tool in McpServer::registered_tools() {
            let area = policy_for(&tool.name).unwrap().area;
            let homes = ToolProfile::ALL.iter().filter(|p| p.admits(&tool)).count();
            if area == ToolArea::Administration {
                assert_eq!(
                    homes, 0,
                    "{} is administration but has a profile",
                    tool.name
                );
            } else {
                assert!(homes > 0, "{} belongs to no profile", tool.name);
            }
        }
        assert!(TOOL_POLICIES
            .iter()
            .any(|p| p.area == ToolArea::Administration));
    }

    #[test]
    fn read_only_and_review_profiles_offer_no_mutation() {
        for tool in McpServer::registered_tools() {
            for profile in [ToolProfile::ReadOnly, ToolProfile::Review] {
                if profile.admits(&tool) {
                    assert_eq!(
                        ToolEffect::of(&tool),
                        ToolEffect::Read,
                        "{profile} offers {}",
                        tool.name
                    );
                }
            }
        }
    }

    #[test]
    fn profiles_keep_to_their_task() {
        assert!(ToolProfile::Authoring.admits(&tool("apply_manifests")));
        assert!(!ToolProfile::Authoring.admits(&tool("delete_entity")));
        assert!(ToolProfile::Repair.admits(&tool("delete_entity")));
        assert!(ToolProfile::Repair.admits(&tool("retarget_references")));
        assert!(ToolProfile::Repair.admits(&tool("merge_branch")));
        assert!(!ToolProfile::Authoring.admits(&tool("merge_branch")));
        assert!(!ToolProfile::Repair.admits(&tool("apply_source_run")));
        assert!(ToolProfile::SourceIntegration.admits(&tool("apply_source_run")));
        assert!(!ToolProfile::SourceIntegration.admits(&tool("put_entity")));
        assert!(ToolProfile::Review.admits(&tool("diff_branch")));
        assert!(!ToolProfile::Review.admits(&tool("list_source_runs")));
        assert!(ToolProfile::ReadOnly.admits(&tool("list_source_runs")));
        for &profile in ToolProfile::ALL {
            assert!(!profile.admits(&tool("delete_by_query")), "{profile}");
            assert!(profile.admits(&tool("who_am_i")), "{profile}");
        }
    }
}
