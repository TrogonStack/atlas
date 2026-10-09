use std::fmt;

const RESERVED_ROOTS: &[&str] = &["google", "trogonatlas"];

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProtoPackagePrefixError {
    #[error("package prefix {0:?} is not a dotted protobuf identifier")]
    Malformed(String),
    #[error("package prefix {0:?} is reserved")]
    Reserved(String),
}

/// The protobuf package namespace a type library owns. A file belongs to the
/// library only when its package equals the prefix or sits under `prefix.`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProtoPackagePrefix(String);

impl ProtoPackagePrefix {
    pub fn parse(raw: &str) -> Result<Self, ProtoPackagePrefixError> {
        if !is_dotted_identifier(raw) {
            return Err(ProtoPackagePrefixError::Malformed(raw.to_owned()));
        }
        let root = raw.split('.').next().unwrap_or_default();
        if RESERVED_ROOTS
            .iter()
            .any(|reserved| root.eq_ignore_ascii_case(reserved))
        {
            return Err(ProtoPackagePrefixError::Reserved(raw.to_owned()));
        }
        Ok(Self(raw.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn owns(&self, package: &str) -> bool {
        package == self.0
            || package
                .strip_prefix(self.0.as_str())
                .is_some_and(|rest| rest.starts_with('.'))
    }

    pub fn overlaps(&self, other: &Self) -> bool {
        self.owns(&other.0) || other.owns(&self.0)
    }
}

impl fmt::Display for ProtoPackagePrefix {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

pub(crate) fn is_dotted_identifier(raw: &str) -> bool {
    !raw.is_empty()
        && raw.split('.').all(|part| {
            let mut bytes = part.bytes();
            bytes
                .next()
                .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
                && bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
        })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn prefix(raw: &str) -> ProtoPackagePrefix {
        ProtoPackagePrefix::parse(raw).unwrap()
    }

    #[test]
    fn owns_itself_and_dotted_children_only() {
        let p = prefix("acme.orders");
        assert!(p.owns("acme.orders"));
        assert!(p.owns("acme.orders.v1"));
        assert!(!p.owns("acme.orders_v2"));
        assert!(!p.owns("acme.ordersv1"));
        assert!(!p.owns("acme"));
    }

    #[test]
    fn overlap_is_symmetric_on_the_dotted_boundary() {
        assert!(prefix("acme").overlaps(&prefix("acme.orders")));
        assert!(prefix("acme.orders").overlaps(&prefix("acme")));
        assert!(!prefix("acme.orders").overlaps(&prefix("acme.ordersx")));
    }

    #[test]
    fn rejects_reserved_roots_and_malformed_prefixes() {
        for raw in ["google", "google.type", "trogonatlas.x", "TrogonAtlas"] {
            assert_eq!(
                ProtoPackagePrefix::parse(raw),
                Err(ProtoPackagePrefixError::Reserved(raw.into()))
            );
        }
        for raw in ["", "acme..orders", "1acme", "acme-orders", ".acme"] {
            assert_eq!(
                ProtoPackagePrefix::parse(raw),
                Err(ProtoPackagePrefixError::Malformed(raw.into()))
            );
        }
    }
}
