use std::fmt;

const MAX_PATH_BYTES: usize = 512;
const RESERVED_DIRS: &[&str] = &["google/protobuf/", "trogonatlas/"];

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProtoPathError {
    #[error("proto path is empty")]
    Empty,
    #[error("proto path {0:?} is longer than {MAX_PATH_BYTES} bytes")]
    TooLong(String),
    #[error("proto path {0:?} must end in .proto")]
    NotProto(String),
    #[error("proto path {0:?} must be relative, with non-empty segments and no . or .. segments")]
    NotRelative(String),
    #[error(
        "proto path {0:?} may only use ASCII letters, digits, '_', '-', '.' and '/' separators"
    )]
    InvalidCharacter(String),
    #[error("proto path {0:?} is under a reserved directory")]
    Reserved(String),
}

/// The import path a tenant file is known by, e.g. `acme/orders/v1/orders.proto`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProtoPath(String);

impl ProtoPath {
    pub fn parse(raw: &str) -> Result<Self, ProtoPathError> {
        if raw.is_empty() {
            return Err(ProtoPathError::Empty);
        }
        if raw.len() > MAX_PATH_BYTES {
            return Err(ProtoPathError::TooLong(raw.to_owned()));
        }
        if !raw
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b'/'))
        {
            return Err(ProtoPathError::InvalidCharacter(raw.to_owned()));
        }
        if raw
            .split('/')
            .any(|segment| segment.is_empty() || segment == "." || segment == "..")
        {
            return Err(ProtoPathError::NotRelative(raw.to_owned()));
        }
        if !raw
            .strip_suffix(".proto")
            .is_some_and(|stem| !stem.is_empty() && !stem.ends_with('/'))
        {
            return Err(ProtoPathError::NotProto(raw.to_owned()));
        }
        if is_reserved_import(raw) {
            return Err(ProtoPathError::Reserved(raw.to_owned()));
        }
        Ok(Self(raw.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ProtoPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

pub(crate) fn is_reserved_import(path: &str) -> bool {
    RESERVED_DIRS.iter().any(|dir| path.starts_with(dir))
}

pub(crate) fn is_atlas_import(path: &str) -> bool {
    path.starts_with("trogonatlas/")
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn accepts_nested_relative_paths() {
        let p = ProtoPath::parse("acme/orders/v1/orders.proto").unwrap();
        assert_eq!(p.as_str(), "acme/orders/v1/orders.proto");
    }

    #[test]
    fn rejects_paths_that_escape_or_are_absolute() {
        for raw in [
            "/acme/a.proto",
            "acme/../a.proto",
            "./a.proto",
            "acme//a.proto",
        ] {
            assert_eq!(
                ProtoPath::parse(raw),
                Err(ProtoPathError::NotRelative(raw.into())),
                "{raw}"
            );
        }
    }

    #[test]
    fn rejects_non_proto_and_odd_characters() {
        assert_eq!(
            ProtoPath::parse("acme/a.txt"),
            Err(ProtoPathError::NotProto("acme/a.txt".into()))
        );
        assert_eq!(
            ProtoPath::parse(".proto"),
            Err(ProtoPathError::NotProto(".proto".into()))
        );
        assert_eq!(
            ProtoPath::parse("acme\\a.proto"),
            Err(ProtoPathError::InvalidCharacter("acme\\a.proto".into()))
        );
        assert_eq!(ProtoPath::parse(""), Err(ProtoPathError::Empty));
    }

    #[test]
    fn rejects_reserved_directories() {
        for raw in [
            "google/protobuf/any.proto",
            "trogonatlas/eventmodel/v1alpha1/x.proto",
        ] {
            assert_eq!(
                ProtoPath::parse(raw),
                Err(ProtoPathError::Reserved(raw.into()))
            );
        }
    }
}
