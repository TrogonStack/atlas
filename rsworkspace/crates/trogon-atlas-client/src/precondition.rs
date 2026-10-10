//! The revisions a decision was made against, and the typed refusals a caller
//! gets when the server no longer holds them. Every surface (trogon-atlas, MCP)
//! downcasts `StalePlan` / `StaleResolution` out of an `anyhow::Error` to tell
//! the agent to read fresh state and plan again, instead of reporting a
//! generic failure.

use std::{fmt, str::FromStr};

use anyhow::{bail, Result};
use serde_json::{json, Value};
use trogon_atlas_proto as pb;

/// Opaque server revision of one stored entity (a KV revision on baseline, a
/// delta row revision on a branch).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Etag(String);

impl Etag {
    pub fn new(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        if value.is_empty() {
            bail!("etag must not be empty");
        }
        if value.contains([',', '=']) || value.chars().any(char::is_whitespace) {
            bail!("etag {value:?} contains a reserved character");
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Etag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// What a caller observed at one key: nothing, or one specific revision.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub enum Revision {
    #[default]
    Absent,
    At(Etag),
}

impl Revision {
    const ABSENT: &'static str = "absent";

    /// Decode the wire form, where the empty string means absent.
    pub fn from_wire(value: &str) -> Result<Self> {
        if value.is_empty() {
            Ok(Self::Absent)
        } else {
            Etag::new(value).map(Self::At)
        }
    }

    #[must_use]
    pub fn to_wire(&self) -> String {
        match self {
            Self::Absent => String::new(),
            Self::At(etag) => etag.as_str().to_string(),
        }
    }

    #[must_use]
    pub fn to_json(&self) -> Value {
        match self {
            Self::Absent => Value::Null,
            Self::At(etag) => Value::String(etag.as_str().to_string()),
        }
    }
}

impl fmt::Display for Revision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Absent => f.write_str(Self::ABSENT),
            Self::At(etag) => etag.fmt(f),
        }
    }
}

impl FromStr for Revision {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        if s == Self::ABSENT {
            Ok(Self::Absent)
        } else {
            Etag::new(s).map(Self::At)
        }
    }
}

/// A planned write refused because the entity it was planned against moved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StalePlan {
    pub entity: pb::EntityKey,
    pub expected: Revision,
    pub actual: Revision,
}

impl StalePlan {
    pub fn from_wire(failure: &pb::PreconditionFailure) -> Result<Self> {
        let Some(subject) = failure.subject.as_ref() else {
            bail!("precondition failure did not name an entity");
        };
        let Some(id) = subject.id.as_ref() else {
            bail!("precondition failure did not carry an id");
        };
        let kind = pb::EntityKind::try_from(subject.kind)
            .map_err(|_| anyhow::anyhow!("unknown entity kind {}", subject.kind))?;
        Ok(Self {
            entity: pb::EntityKey::new(kind, id),
            expected: Revision::from_wire(&failure.expected_etag)?,
            actual: Revision::from_wire(&failure.actual_etag)?,
        })
    }

    #[must_use]
    pub fn to_json(&self) -> Value {
        json!({
            "entity": self.entity.to_string(),
            "expected_etag": self.expected.to_json(),
            "actual_etag": self.actual.to_json(),
        })
    }
}

impl fmt::Display for StalePlan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "stale plan for {}: planned against revision {}, server now holds {}; plan again from fresh state",
            self.entity, self.expected, self.actual
        )
    }
}

impl std::error::Error for StalePlan {}

/// The three revisions a branch conflict was classified from. A resolution
/// carries the state its author inspected so the server can refuse it once
/// any side has moved.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BranchEntryState {
    pub base: Revision,
    pub ours: Revision,
    pub theirs: Revision,
}

impl BranchEntryState {
    pub fn from_wire(state: &pb::BranchEntryState) -> Result<Self> {
        Ok(Self {
            base: Revision::from_wire(&state.base_etag)?,
            ours: Revision::from_wire(&state.ours_etag)?,
            theirs: Revision::from_wire(&state.theirs_etag)?,
        })
    }

    #[must_use]
    pub fn to_wire(&self) -> pb::BranchEntryState {
        pb::BranchEntryState {
            base_etag: self.base.to_wire(),
            ours_etag: self.ours.to_wire(),
            theirs_etag: self.theirs.to_wire(),
        }
    }

    #[must_use]
    pub fn to_json(&self) -> Value {
        json!({
            "base_etag": self.base.to_json(),
            "ours_etag": self.ours.to_json(),
            "theirs_etag": self.theirs.to_json(),
        })
    }
}

impl fmt::Display for BranchEntryState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "base={},ours={},theirs={}",
            self.base, self.ours, self.theirs
        )
    }
}

impl FromStr for BranchEntryState {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        let (mut base, mut ours, mut theirs) = (None, None, None);
        for part in s.split(',') {
            let Some((name, value)) = part.split_once('=') else {
                bail!("expected base=<rev>,ours=<rev>,theirs=<rev>, got {s:?}");
            };
            let slot = match name.trim() {
                "base" => &mut base,
                "ours" => &mut ours,
                "theirs" => &mut theirs,
                other => bail!("unknown side {other:?} in {s:?}"),
            };
            if slot.replace(value.trim().parse::<Revision>()?).is_some() {
                bail!("side {name:?} given twice in {s:?}");
            }
        }
        match (base, ours, theirs) {
            (Some(base), Some(ours), Some(theirs)) => Ok(Self { base, ours, theirs }),
            _ => bail!("expected base=<rev>,ours=<rev>,theirs=<rev>, got {s:?}"),
        }
    }
}

/// A conflict resolution refused because the entry it decided about moved
/// after it was inspected. `actual` is `None` when the branch no longer holds
/// a delta for the key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaleResolution {
    pub branch: String,
    pub entity: pb::EntityKey,
    pub expected: BranchEntryState,
    pub actual: Option<BranchEntryState>,
}

impl StaleResolution {
    #[must_use]
    pub fn to_json(&self) -> Value {
        json!({
            "branch": self.branch,
            "entity": self.entity.to_string(),
            "expected_state": self.expected.to_json(),
            "actual_state": self.actual.as_ref().map(BranchEntryState::to_json),
        })
    }
}

impl fmt::Display for StaleResolution {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "stale resolution for {} on branch {:?}: decided against {}, ",
            self.entity, self.branch, self.expected
        )?;
        match &self.actual {
            Some(actual) => write!(f, "entry is now {actual}")?,
            None => f.write_str("branch no longer holds an entry for it")?,
        }
        f.write_str("; diff the branch again and decide from the current entry")
    }
}

impl std::error::Error for StaleResolution {}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn at(etag: &str) -> Revision {
        Revision::At(Etag::new(etag).unwrap())
    }

    #[test]
    fn empty_wire_etag_means_absent() {
        assert_eq!(Revision::from_wire("").unwrap(), Revision::Absent);
        assert_eq!(Revision::from_wire("7").unwrap(), at("7"));
        assert_eq!(at("7").to_wire(), "7");
        assert_eq!(Revision::Absent.to_wire(), "");
    }

    #[test]
    fn branch_entry_state_round_trips_through_its_token() {
        let state = BranchEntryState {
            base: at("3"),
            ours: at("9"),
            theirs: Revision::Absent,
        };
        let token = state.to_string();
        assert_eq!(token, "base=3,ours=9,theirs=absent");
        assert_eq!(token.parse::<BranchEntryState>().unwrap(), state);
    }

    #[test]
    fn branch_entry_state_token_rejects_missing_or_repeated_sides() {
        assert!("base=1,ours=2".parse::<BranchEntryState>().is_err());
        assert!("base=1,ours=2,theirs=3,base=4"
            .parse::<BranchEntryState>()
            .is_err());
        assert!("base=1,ours=2,mine=3".parse::<BranchEntryState>().is_err());
        assert!("base=,ours=2,theirs=3".parse::<BranchEntryState>().is_err());
    }
}
