use std::fmt;

use crate::package::is_dotted_identifier;

const DEFAULT_HOST: &str = "type.googleapis.com";

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("type url {0:?} must look like <host>/<fully.qualified.MessageName>")]
pub struct TypeUrlError(String);

/// A `google.protobuf.Any` type url. Only the part after the last `/` names
/// the message; the host is carried but never dereferenced.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TypeUrl {
    raw: String,
    name_start: usize,
}

impl TypeUrl {
    pub fn parse(raw: &str) -> Result<Self, TypeUrlError> {
        let Some(slash) = raw.rfind('/') else {
            return Err(TypeUrlError(raw.to_owned()));
        };
        let name = &raw[slash + 1..];
        if slash == 0 || !is_dotted_identifier(name) {
            return Err(TypeUrlError(raw.to_owned()));
        }
        Ok(Self {
            raw: raw.to_owned(),
            name_start: slash + 1,
        })
    }

    pub fn for_message(full_name: &str) -> Result<Self, TypeUrlError> {
        Self::parse(&format!("{DEFAULT_HOST}/{full_name}"))
    }

    pub fn full_name(&self) -> &str {
        &self.raw[self.name_start..]
    }

    pub fn as_str(&self) -> &str {
        &self.raw
    }
}

impl fmt::Display for TypeUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.raw)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn names_the_message_after_the_last_slash() {
        let url = TypeUrl::parse("type.googleapis.com/acme.orders.v1.OrderPlaced").unwrap();
        assert_eq!(url.full_name(), "acme.orders.v1.OrderPlaced");
        let custom = TypeUrl::parse("example.com/types/acme.Thing").unwrap();
        assert_eq!(custom.full_name(), "acme.Thing");
    }

    #[test]
    fn for_message_uses_the_conventional_host() {
        let url = TypeUrl::for_message("acme.Thing").unwrap();
        assert_eq!(url.as_str(), "type.googleapis.com/acme.Thing");
    }

    #[test]
    fn rejects_urls_without_a_message_name() {
        for raw in [
            "acme.Thing",
            "/acme.Thing",
            "type.googleapis.com/",
            "h/acme..X",
        ] {
            assert!(TypeUrl::parse(raw).is_err(), "{raw}");
        }
    }
}
