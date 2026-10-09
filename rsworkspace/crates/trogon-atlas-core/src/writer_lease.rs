//! Vocabulary for the single-writer posture: which role a server process
//! runs with, and the monotonically increasing epoch a writer lease hands
//! out. See `docs/explanation/single-writer.md` for the full design.

use std::{fmt, str::FromStr};

/// Which writer role one server process runs with.
///
/// `Writer` and `Standby` both participate in acquiring the writer lease
/// (see `trogon-atlas-store::writer_lease`); which one actually holds it at
/// any moment is runtime state, not configuration, so a process configured
/// `Standby` becomes the live writer the moment it takes over an expired
/// lease. `Reader` never participates: it only ever serves reads and
/// refuses every mutation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WriterRole {
    Writer,
    Standby,
    Reader,
}

impl WriterRole {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Writer => "writer",
            Self::Standby => "standby",
            Self::Reader => "reader",
        }
    }

    /// Whether this role ever participates in the writer lease (attempts to
    /// acquire or renew it). `Reader` never does.
    #[must_use]
    pub fn participates_in_lease(self) -> bool {
        !matches!(self, Self::Reader)
    }
}

impl fmt::Display for WriterRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// `s` was not one of `writer`, `standby`, `reader`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseWriterRoleError(String);

impl fmt::Display for ParseWriterRoleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:?} is not a writer role; expected one of writer, standby, reader",
            self.0
        )
    }
}

impl std::error::Error for ParseWriterRoleError {}

impl FromStr for WriterRole {
    type Err = ParseWriterRoleError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "writer" => Ok(Self::Writer),
            "standby" => Ok(Self::Standby),
            "reader" => Ok(Self::Reader),
            other => Err(ParseWriterRoleError(other.to_owned())),
        }
    }
}

/// A writer lease epoch: strictly increases every time the lease changes
/// hands, never on a renewal by the same holder. A write stamped with an
/// epoch older than the lease's current one came from a writer that has
/// since been fenced out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Epoch(u64);

impl Epoch {
    /// Before any process has ever claimed the lease.
    pub const NONE: Self = Self(0);

    #[must_use]
    pub fn new(value: u64) -> Self {
        Self(value)
    }

    #[must_use]
    pub fn get(self) -> u64 {
        self.0
    }

    /// The epoch the next holder takes over with.
    #[must_use]
    pub fn next(self) -> Self {
        Self(self.0 + 1)
    }
}

impl fmt::Display for Epoch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn writer_role_round_trips_through_its_string_form() {
        for role in [WriterRole::Writer, WriterRole::Standby, WriterRole::Reader] {
            assert_eq!(role.as_str().parse::<WriterRole>().unwrap(), role);
            assert_eq!(role.to_string(), role.as_str());
        }
    }

    #[test]
    fn writer_role_rejects_unknown_strings() {
        let err = "bogus".parse::<WriterRole>().unwrap_err();
        assert!(err.to_string().contains("bogus"));
    }

    #[test]
    fn only_reader_does_not_participate_in_the_lease() {
        assert!(WriterRole::Writer.participates_in_lease());
        assert!(WriterRole::Standby.participates_in_lease());
        assert!(!WriterRole::Reader.participates_in_lease());
    }

    #[test]
    fn epoch_next_strictly_increases() {
        let first = Epoch::NONE.next();
        let second = first.next();
        assert_eq!(first.get(), 1);
        assert_eq!(second.get(), 2);
        assert!(second > first);
    }

    #[test]
    fn epoch_displays_its_bare_number() {
        assert_eq!(Epoch::new(7).to_string(), "7");
    }
}
