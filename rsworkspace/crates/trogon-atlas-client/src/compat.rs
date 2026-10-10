//! Mixed-version gate: every surface asks `GetServerInfo` before mutating and
//! refuses when the server would read the request differently than the
//! client means it (see `docs/explanation/wire-compatibility.md`).

use std::fmt;

use trogon_atlas_proto as pb;

use crate::client::Client;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capability {
    Mutations,
    ValidateOnly,
    BranchScopedRequests,
    StatePreconditions,
    WhoAmI,
    TypeLibraries,
}

impl Capability {
    fn advertised_by(self, features: &pb::get_server_info_response::Features) -> bool {
        match self {
            Capability::Mutations => features.mutations,
            Capability::ValidateOnly => features.validate_only,
            Capability::BranchScopedRequests => features.branch_scoped_requests,
            Capability::StatePreconditions => features.state_preconditions,
            Capability::WhoAmI => features.who_am_i,
            Capability::TypeLibraries => features.type_libraries,
        }
    }

    #[must_use]
    pub fn feature_flag(self) -> &'static str {
        match self {
            Capability::Mutations => "mutations",
            Capability::ValidateOnly => "validate_only",
            Capability::BranchScopedRequests => "branch_scoped_requests",
            Capability::StatePreconditions => "state_preconditions",
            Capability::WhoAmI => "who_am_i",
            Capability::TypeLibraries => "type_libraries",
        }
    }
}

impl fmt::Display for Capability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.feature_flag())
    }
}

/// What the caller is about to ask the server to do, which decides the
/// capabilities the server must advertise first.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MutationIntent {
    /// The request relies on the server not persisting (validate_only,
    /// DRY_RUN, dry_run).
    pub validate_only: bool,
    /// The client sends `x-trogon-atlas-branch`, so a server that ignores it
    /// would write to baseline instead.
    pub branch_scoped: bool,
    /// The request is a decision planned from earlier reads (apply, conflict
    /// resolution), so the server must report and enforce the revisions it
    /// was planned against; one that does not would let it overwrite a
    /// concurrent change.
    pub state_preconditions: bool,
}

impl MutationIntent {
    #[must_use]
    pub fn required_capabilities(self) -> Vec<Capability> {
        let mut required = vec![Capability::Mutations];
        if self.validate_only {
            required.push(Capability::ValidateOnly);
        }
        if self.branch_scoped {
            required.push(Capability::BranchScopedRequests);
        }
        if self.state_preconditions {
            required.push(Capability::StatePreconditions);
        }
        required
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CompatibilityError {
    #[error("server discovery (GetServerInfo) failed: {0}")]
    Discovery(#[source] Box<tonic::Status>),

    #[error(
        "server implements schema {server:?} but this client speaks {client:?}; refusing to mutate"
    )]
    SchemaMismatch {
        server: String,
        client: &'static str,
    },

    #[error(
        "server contract revision {server} is older than revision {minimum} this client requires; upgrade the server before mutating"
    )]
    ServerTooOld { server: u32, minimum: u32 },

    #[error(
        "client contract revision {client} is older than revision {minimum} the server requires; upgrade this client before mutating"
    )]
    ClientTooOld { client: u32, minimum: u32 },

    #[error(
        "server does not advertise the {0} capability (GetServerInfo.features.{0}); refusing to mutate"
    )]
    MissingCapability(Capability),
}

/// Pure decision over a discovery response, so every unsupported
/// combination is testable without a server.
pub fn check(
    info: &pb::GetServerInfoResponse,
    intent: MutationIntent,
) -> Result<(), CompatibilityError> {
    if info.schema_version != pb::SCHEMA_VERSION {
        return Err(CompatibilityError::SchemaMismatch {
            server: info.schema_version.clone(),
            client: pb::SCHEMA_VERSION,
        });
    }
    if info.contract_revision < pb::MIN_SERVER_CONTRACT_REVISION {
        return Err(CompatibilityError::ServerTooOld {
            server: info.contract_revision,
            minimum: pb::MIN_SERVER_CONTRACT_REVISION,
        });
    }
    if pb::CONTRACT_REVISION < info.min_client_contract_revision {
        return Err(CompatibilityError::ClientTooOld {
            client: pb::CONTRACT_REVISION,
            minimum: info.min_client_contract_revision,
        });
    }
    let absent = pb::get_server_info_response::Features::default();
    let features = info.features.as_ref().unwrap_or(&absent);
    match intent
        .required_capabilities()
        .into_iter()
        .find(|capability| !capability.advertised_by(features))
    {
        Some(missing) => Err(CompatibilityError::MissingCapability(missing)),
        None => Ok(()),
    }
}

/// Ask the server what it supports and refuse before the mutation is sent.
pub async fn ensure_can_mutate(
    client: &mut Client,
    intent: MutationIntent,
) -> Result<(), CompatibilityError> {
    let info = client
        .get_server_info(pb::GetServerInfoRequest {})
        .await
        .map_err(|status| CompatibilityError::Discovery(Box::new(status)))?
        .into_inner();
    check(&info, intent)
}

/// Whether the server advertises `capability`. A server too old to know the
/// flag reads as not advertising it.
pub async fn advertises(
    client: &mut Client,
    capability: Capability,
) -> Result<bool, CompatibilityError> {
    let info = client
        .get_server_info(pb::GetServerInfoRequest {})
        .await
        .map_err(|status| CompatibilityError::Discovery(Box::new(status)))?
        .into_inner();
    Ok(info
        .features
        .as_ref()
        .is_some_and(|features| capability.advertised_by(features)))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn current_server() -> pb::GetServerInfoResponse {
        pb::GetServerInfoResponse {
            schema_version: pb::SCHEMA_VERSION.into(),
            server_version: "test".into(),
            features: Some(pb::get_server_info_response::Features {
                mutations: true,
                validate_only: true,
                branch_scoped_requests: true,
                state_preconditions: true,
                ..Default::default()
            }),
            limits: None,
            contract_revision: pb::CONTRACT_REVISION,
            min_client_contract_revision: pb::MIN_CLIENT_CONTRACT_REVISION,
            writer_status: None,
        }
    }

    fn every_intent() -> [MutationIntent; 5] {
        [
            MutationIntent::default(),
            MutationIntent {
                validate_only: true,
                ..Default::default()
            },
            MutationIntent {
                branch_scoped: true,
                ..Default::default()
            },
            MutationIntent {
                validate_only: true,
                branch_scoped: true,
                ..Default::default()
            },
            MutationIntent {
                validate_only: true,
                branch_scoped: true,
                state_preconditions: true,
            },
        ]
    }

    #[test]
    fn who_am_i_capability_reflects_the_feature_flag() {
        assert_eq!(Capability::WhoAmI.feature_flag(), "who_am_i");
        let mut features = pb::get_server_info_response::Features::default();
        assert!(!Capability::WhoAmI.advertised_by(&features));
        features.who_am_i = true;
        assert!(Capability::WhoAmI.advertised_by(&features));
    }

    #[test]
    fn type_libraries_capability_reflects_the_feature_flag() {
        assert_eq!(Capability::TypeLibraries.feature_flag(), "type_libraries");
        let mut features = pb::get_server_info_response::Features::default();
        assert!(!Capability::TypeLibraries.advertised_by(&features));
        features.type_libraries = true;
        assert!(Capability::TypeLibraries.advertised_by(&features));
    }

    #[test]
    fn current_server_accepts_every_mutation_intent() {
        for intent in every_intent() {
            check(&current_server(), intent).unwrap();
        }
    }

    #[test]
    fn server_that_predates_contract_revisions_is_refused() {
        let info = pb::GetServerInfoResponse {
            schema_version: pb::SCHEMA_VERSION.into(),
            features: Some(pb::get_server_info_response::Features {
                search: true,
                change_feed: true,
                ..Default::default()
            }),
            ..Default::default()
        };
        assert!(matches!(
            check(&info, MutationIntent::default()),
            Err(CompatibilityError::ServerTooOld { server: 0, .. })
        ));
    }

    #[test]
    fn other_schema_is_refused() {
        let info = pb::GetServerInfoResponse {
            schema_version: "eventmodel.v2".into(),
            ..current_server()
        };
        let err = check(&info, MutationIntent::default()).unwrap_err();
        assert!(matches!(err, CompatibilityError::SchemaMismatch { .. }));
        assert!(err.to_string().contains("eventmodel.v2"));
    }

    #[test]
    fn server_requiring_newer_client_is_refused() {
        let info = pb::GetServerInfoResponse {
            contract_revision: pb::CONTRACT_REVISION + 1,
            min_client_contract_revision: pb::CONTRACT_REVISION + 1,
            ..current_server()
        };
        assert!(matches!(
            check(&info, MutationIntent::default()),
            Err(CompatibilityError::ClientTooOld { .. })
        ));
    }

    #[test]
    fn newer_server_that_still_accepts_this_client_is_allowed() {
        let info = pb::GetServerInfoResponse {
            contract_revision: pb::CONTRACT_REVISION + 3,
            ..current_server()
        };
        for intent in every_intent() {
            check(&info, intent).unwrap();
        }
    }

    #[test]
    fn dry_run_against_server_without_validate_only_is_refused() {
        let mut info = current_server();
        info.features.as_mut().unwrap().validate_only = false;
        check(&info, MutationIntent::default()).unwrap();
        let err = check(
            &info,
            MutationIntent {
                validate_only: true,
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(matches!(
            err,
            CompatibilityError::MissingCapability(Capability::ValidateOnly)
        ));
        assert!(err.to_string().contains("features.validate_only"));
    }

    #[test]
    fn branch_write_against_server_ignoring_branch_header_is_refused() {
        let mut info = current_server();
        info.features.as_mut().unwrap().branch_scoped_requests = false;
        check(&info, MutationIntent::default()).unwrap();
        assert!(matches!(
            check(
                &info,
                MutationIntent {
                    branch_scoped: true,
                    ..Default::default()
                },
            ),
            Err(CompatibilityError::MissingCapability(
                Capability::BranchScopedRequests
            ))
        ));
    }

    #[test]
    fn planned_write_against_server_without_state_preconditions_is_refused() {
        let mut info = current_server();
        info.features.as_mut().unwrap().state_preconditions = false;
        check(&info, MutationIntent::default()).unwrap();
        let err = check(
            &info,
            MutationIntent {
                state_preconditions: true,
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(matches!(
            err,
            CompatibilityError::MissingCapability(Capability::StatePreconditions)
        ));
        assert!(err.to_string().contains("features.state_preconditions"));
    }

    #[test]
    fn read_only_server_refuses_every_mutation() {
        let mut info = current_server();
        info.features.as_mut().unwrap().mutations = false;
        for intent in every_intent() {
            assert!(matches!(
                check(&info, intent),
                Err(CompatibilityError::MissingCapability(Capability::Mutations))
            ));
        }
    }

    #[test]
    fn missing_features_block_is_refused() {
        let info = pb::GetServerInfoResponse {
            features: None,
            ..current_server()
        };
        assert!(matches!(
            check(&info, MutationIntent::default()),
            Err(CompatibilityError::MissingCapability(Capability::Mutations))
        ));
    }
}
