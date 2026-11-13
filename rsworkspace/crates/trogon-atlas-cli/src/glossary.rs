use anyhow::Result;
use clap::Subcommand;
use serde_json::Value;
use trogon_atlas_client::{client::Client, glossary};
use trogon_atlas_proto as pb;

#[derive(Subcommand)]
pub enum GlossaryCmd {
    /// List compact contextual vocabulary with exact references and publication state.
    List {
        #[arg(long)]
        request: String,
    },
    /// Read one exact glossary Term, its definition, review and source freshness.
    Get {
        #[arg(long)]
        request: String,
    },
    /// Inspect retained source Documents and their exact byte and incarnation pins.
    Sources {
        #[arg(long)]
        request: String,
    },
    /// Record an explicit User review decision under the current Term etag.
    Review {
        #[arg(long)]
        request: String,
    },
    /// Compare exact glossary selections without inferring a language ruling.
    Compare {
        #[arg(long)]
        request: String,
    },
    /// Check supplied revision-bound text against scoped terminology policies.
    Check {
        #[arg(long)]
        request: String,
    },
    /// Inspect source, generation and review freshness without changing publication.
    Refresh {
        #[arg(long)]
        request: String,
    },
}

pub enum PreparedGlossaryCommand {
    List(pb::ListGlossaryEntriesRequest),
    Get(pb::GetGlossaryTermRequest),
    Sources(pb::InspectGlossarySourcesRequest),
    Review(pb::ReviewGlossaryTermRequest),
    Compare(pb::CompareGlossariesRequest),
    Check(pb::CheckGlossaryUsageRequest),
    Refresh(pb::InspectGlossaryRefreshRequest),
}

impl GlossaryCmd {
    pub fn prepare(self) -> Result<PreparedGlossaryCommand> {
        let parse = |request: &str| -> Result<Value> {
            serde_json::from_str(request).map_err(|error| {
                trogon_atlas_client::failure::invalid_input(format!(
                    "invalid request JSON: {error}"
                ))
            })
        };
        Ok(match self {
            Self::List { request } => PreparedGlossaryCommand::List(glossary::parse_request(
                "ListGlossaryEntriesRequest",
                &parse(&request)?,
            )?),
            Self::Get { request } => PreparedGlossaryCommand::Get(glossary::parse_request(
                "GetGlossaryTermRequest",
                &parse(&request)?,
            )?),
            Self::Sources { request } => PreparedGlossaryCommand::Sources(glossary::parse_request(
                "InspectGlossarySourcesRequest",
                &parse(&request)?,
            )?),
            Self::Review { request } => PreparedGlossaryCommand::Review(glossary::parse_request(
                "ReviewGlossaryTermRequest",
                &parse(&request)?,
            )?),
            Self::Compare { request } => PreparedGlossaryCommand::Compare(glossary::parse_request(
                "CompareGlossariesRequest",
                &parse(&request)?,
            )?),
            Self::Check { request } => PreparedGlossaryCommand::Check(glossary::parse_request(
                "CheckGlossaryUsageRequest",
                &parse(&request)?,
            )?),
            Self::Refresh { request } => PreparedGlossaryCommand::Refresh(glossary::parse_request(
                "InspectGlossaryRefreshRequest",
                &parse(&request)?,
            )?),
        })
    }
}

impl PreparedGlossaryCommand {
    pub fn mutation_preview(&self) -> Option<bool> {
        match self {
            Self::Review(request) => Some(request.validate_only),
            _ => None,
        }
    }

    pub async fn execute(self, client: &mut Client) -> Result<Value> {
        match self {
            Self::List(request) => glossary::list_glossary_entries(client, request).await,
            Self::Get(request) => glossary::get_glossary_term(client, request).await,
            Self::Sources(request) => glossary::inspect_glossary_sources(client, request).await,
            Self::Review(request) => glossary::review_glossary_term(client, request).await,
            Self::Compare(request) => glossary::compare_glossaries(client, request).await,
            Self::Check(request) => glossary::check_glossary_usage(client, request).await,
            Self::Refresh(request) => glossary::inspect_glossary_refresh(client, request).await,
        }
    }
}
