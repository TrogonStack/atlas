//! Namespace scope: the second authorization axis.
//!
//! `required_role` answers "may this caller call this RPC at all". It cannot
//! answer "may this caller write *this* part of the model", because the gRPC
//! path carries no namespace. That question needs the request body, so it is
//! enforced in the handlers, at the same boundary that already resolves
//! branch context.
//!
//! Two different things are expressed with the same patterns and must not be
//! confused:
//!
//! - [`NamespaceScope`] is an allow-list carried by a principal. **Empty means
//!   unrestricted**, so a registry that predates this field keeps working.
//! - [`BaselineProtection::Namespaces`] is a protected-list carried by the
//!   server. It is never empty by construction: an empty list collapses to
//!   [`BaselineProtection::Off`].

use std::{collections::BTreeSet, sync::Arc};

/// One entry in a namespace allow-list or protected-list.
///
/// Two forms only. `orders` matches that namespace and nothing else;
/// `shop-*` matches every namespace starting with `shop-`. A bare `*`
/// matches everything, which is a prefix of length zero.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NamespacePattern {
    Exact(String),
    Prefix(String),
}

impl NamespacePattern {
    /// Parse one pattern.
    ///
    /// # Errors
    ///
    /// Returns a human-readable message when the pattern is empty, puts `*`
    /// anywhere but last, or (outside the wildcard) uses characters that
    /// could never appear in a namespace.
    pub fn parse(raw: &str) -> Result<Self, String> {
        if raw.is_empty() {
            return Err("namespace pattern must not be empty".to_owned());
        }
        if let Some(prefix) = raw.strip_suffix('*') {
            if prefix.contains('*') {
                return Err(format!(
                    "namespace pattern {raw:?}: '*' may appear only once, as the last character",
                ));
            }
            if prefix.is_empty() {
                return Ok(Self::Prefix(String::new()));
            }
            // The prefix is a fragment of a namespace rather than a whole
            // one, but every character in it must still be a character a
            // namespace could contain, or the pattern can never match.
            trogon_atlas_core::validate_id_component(prefix)
                .map_err(|e| format!("namespace pattern {raw:?}: prefix {e}"))?;
            return Ok(Self::Prefix(prefix.to_owned()));
        }
        if raw.contains('*') {
            return Err(format!(
                "namespace pattern {raw:?}: '*' is only allowed as the last character",
            ));
        }
        trogon_atlas_core::validate_id_component(raw)
            .map_err(|e| format!("namespace pattern {raw:?} {e}"))?;
        Ok(Self::Exact(raw.to_owned()))
    }

    #[must_use]
    pub fn matches(&self, namespace: &str) -> bool {
        match self {
            Self::Exact(want) => namespace == want,
            Self::Prefix(prefix) => namespace.starts_with(prefix.as_str()),
        }
    }
}

impl std::fmt::Display for NamespacePattern {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Exact(value) => f.write_str(value),
            Self::Prefix(prefix) => write!(f, "{prefix}*"),
        }
    }
}

/// A parsed, immutable list of patterns. Cheap to clone (one `Arc` bump), so
/// it can ride along on every `Principal`.
///
/// Deliberately says nothing about what an empty list means: that meaning
/// belongs to whoever holds the list, and the two holders disagree.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NamespacePatterns(Arc<[NamespacePattern]>);

impl NamespacePatterns {
    /// Parse every pattern, failing on the first bad one.
    ///
    /// # Errors
    ///
    /// Propagates the message from [`NamespacePattern::parse`].
    pub fn parse<S: AsRef<str>>(raw: &[S]) -> Result<Self, String> {
        let mut patterns = Vec::with_capacity(raw.len());
        for value in raw {
            patterns.push(NamespacePattern::parse(value.as_ref())?);
        }
        Ok(Self(patterns.into()))
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    #[must_use]
    pub fn matches_any(&self, namespace: &str) -> bool {
        self.0.iter().any(|pattern| pattern.matches(namespace))
    }

    /// Render each pattern back to its source form, e.g. `["orders",
    /// "shop-*"]`. Used where the patterns must travel somewhere other than
    /// a human-readable `Display`, such as a wire response.
    #[must_use]
    pub fn to_strings(&self) -> Vec<String> {
        self.0.iter().map(ToString::to_string).collect()
    }
}

impl std::fmt::Display for NamespacePatterns {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let rendered: Vec<String> = self.0.iter().map(ToString::to_string).collect();
        f.write_str(&rendered.join(", "))
    }
}

/// The namespaces a principal is allowed to write.
///
/// An empty scope is unrestricted. That is the opt-in property the whole
/// feature rests on: a registry written before this field existed parses into
/// an empty scope and behaves exactly as it did before.
///
/// Scope constrains **writes only**. Reading the rest of the model is what
/// makes a bounded context useful to its neighbours; a scoped principal that
/// could not read across seams could not model against them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NamespaceScope(NamespacePatterns);

/// Why a write fell outside a principal's scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutOfScope {
    /// This namespace is not on the allow-list.
    Namespace(String),
    /// The RPC picks its own targets, so no allow-list can bound it.
    Unbounded,
}

impl NamespaceScope {
    #[must_use]
    pub fn unrestricted() -> Self {
        Self(NamespacePatterns::default())
    }

    /// Build a scope from raw registry values.
    ///
    /// # Errors
    ///
    /// Propagates the message from [`NamespacePatterns::parse`].
    pub fn parse<S: AsRef<str>>(raw: &[S]) -> Result<Self, String> {
        Ok(Self(NamespacePatterns::parse(raw)?))
    }

    #[must_use]
    pub fn is_unrestricted(&self) -> bool {
        self.0.is_empty()
    }

    #[must_use]
    pub fn admits(&self, namespace: &str) -> bool {
        self.is_unrestricted() || self.0.matches_any(namespace)
    }

    /// The raw patterns, or an empty list when unrestricted. See
    /// [`NamespacePatterns::to_strings`].
    #[must_use]
    pub fn to_strings(&self) -> Vec<String> {
        self.0.to_strings()
    }

    /// `None` when every target is admitted, otherwise the first reason it is
    /// not. Ordering is deterministic because [`WriteTargets`] keeps its
    /// namespaces sorted, so the same request always names the same offender.
    #[must_use]
    pub fn deny_reason(&self, targets: &WriteTargets) -> Option<OutOfScope> {
        if self.is_unrestricted() {
            return None;
        }
        match targets {
            WriteTargets::Known(namespaces) => namespaces
                .iter()
                .find(|namespace| !self.0.matches_any(namespace))
                .map(|namespace| OutOfScope::Namespace(namespace.clone())),
            WriteTargets::Unbounded => Some(OutOfScope::Unbounded),
        }
    }
}

impl std::fmt::Display for NamespaceScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.is_unrestricted() {
            f.write_str("*")
        } else {
            std::fmt::Display::fmt(&self.0, f)
        }
    }
}

/// The namespaces one mutating RPC is about to write into.
///
/// Most RPCs name their targets in the request. `RetargetReferences` does
/// not: it rewrites whatever happens to reference an entity, anywhere in the
/// model, and the answer is only known after a scan. Modelling that as an
/// empty set would silently authorize it, so it gets its own case and is
/// treated as touching everything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteTargets {
    Known(BTreeSet<String>),
    Unbounded,
}

impl WriteTargets {
    #[must_use]
    pub fn none() -> Self {
        Self::Known(BTreeSet::new())
    }

    #[must_use]
    pub fn of(namespace: &str) -> Self {
        let mut namespaces = BTreeSet::new();
        namespaces.insert(namespace.to_owned());
        Self::Known(namespaces)
    }

    /// Adds one namespace. A no-op on [`WriteTargets::Unbounded`], which is
    /// already as wide as it can get.
    pub fn insert(&mut self, namespace: &str) {
        if let Self::Known(namespaces) = self {
            namespaces.insert(namespace.to_owned());
        }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        matches!(self, Self::Known(namespaces) if namespaces.is_empty())
    }
}

/// Which namespaces refuse direct baseline writes.
///
/// The global switch this replaces could only say "all" or "none". A model
/// with several teams in it wants neither: the namespace under active review
/// is protected, the sandbox next to it is not.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum BaselineProtection {
    #[default]
    Off,
    All,
    /// Guaranteed non-empty; see [`BaselineProtection::configure`].
    Namespaces(NamespacePatterns),
}

impl BaselineProtection {
    /// Resolve the two operator-facing knobs into one value.
    ///
    /// `--protect-baseline-namespaces` narrows `--protect-baseline` rather
    /// than conflicting with it: naming namespaces is a more specific
    /// statement than the blanket flag, so it wins.
    ///
    /// # Errors
    ///
    /// Propagates the message from [`NamespacePatterns::parse`].
    pub fn configure<S: AsRef<str>>(protect_all: bool, namespaces: &[S]) -> Result<Self, String> {
        let patterns = NamespacePatterns::parse(namespaces)?;
        if !patterns.is_empty() {
            return Ok(Self::Namespaces(patterns));
        }
        Ok(if protect_all { Self::All } else { Self::Off })
    }

    #[must_use]
    pub fn is_off(&self) -> bool {
        matches!(self, Self::Off)
    }

    /// Whether a direct baseline write to `targets` needs an authenticated
    /// Admin. An unbounded write is protected whenever protection is on at
    /// all: it could land in a protected namespace and there is no way to
    /// prove otherwise before the scan.
    #[must_use]
    pub fn protects(&self, targets: &WriteTargets) -> bool {
        match self {
            Self::Off => false,
            Self::All => true,
            Self::Namespaces(patterns) => match targets {
                WriteTargets::Known(namespaces) => namespaces
                    .iter()
                    .any(|namespace| patterns.matches_any(namespace)),
                WriteTargets::Unbounded => true,
            },
        }
    }
}

impl std::fmt::Display for BaselineProtection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Off => f.write_str("off"),
            Self::All => f.write_str("all namespaces"),
            Self::Namespaces(patterns) => std::fmt::Display::fmt(patterns, f),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn scope(patterns: &[&str]) -> NamespaceScope {
        NamespaceScope::parse(patterns).unwrap()
    }

    #[test]
    fn an_empty_scope_admits_everything() {
        let scope = NamespaceScope::unrestricted();
        assert!(scope.is_unrestricted());
        assert!(scope.admits("anything"));
        assert!(scope.deny_reason(&WriteTargets::Unbounded).is_none());
    }

    #[test]
    fn an_exact_pattern_admits_only_that_namespace() {
        let scope = scope(&["orders"]);
        assert!(scope.admits("orders"));
        assert!(!scope.admits("orders-archive"));
        assert!(!scope.admits("billing"));
    }

    #[test]
    fn a_trailing_star_admits_the_prefix() {
        let scope = scope(&["shop-*"]);
        assert!(scope.admits("shop-orders"));
        assert!(scope.admits("shop-"));
        assert!(!scope.admits("shop"));
        assert!(!scope.admits("billing"));
    }

    #[test]
    fn a_bare_star_is_a_zero_length_prefix() {
        let scope = scope(&["*"]);
        assert!(!scope.is_unrestricted(), "the list is not empty");
        assert!(scope.admits("anything"));
        assert!(
            scope.deny_reason(&WriteTargets::Unbounded).is_some(),
            "an explicit list still cannot bound an unbounded write"
        );
    }

    #[test]
    fn a_star_anywhere_but_last_is_rejected() {
        for raw in ["*shop", "sh*op", "sh**", "**"] {
            assert!(
                NamespacePattern::parse(raw).is_err(),
                "accepted ambiguous pattern {raw:?}"
            );
        }
    }

    #[test]
    fn patterns_that_could_never_match_a_namespace_are_rejected() {
        for raw in ["", "a/b", "..", ".hidden", "a b", "shop/*"] {
            assert!(
                NamespacePattern::parse(raw).is_err(),
                "accepted unmatchable pattern {raw:?}"
            );
        }
    }

    #[test]
    fn deny_reason_names_the_first_offending_namespace() {
        let scope = scope(&["orders"]);
        let mut targets = WriteTargets::of("orders");
        targets.insert("zeta");
        targets.insert("billing");
        assert_eq!(
            scope.deny_reason(&targets),
            Some(OutOfScope::Namespace("billing".to_owned())),
            "targets are sorted, so the offender is stable across calls"
        );
    }

    #[test]
    fn a_write_touching_nothing_is_admitted_by_any_scope() {
        assert!(scope(&["orders"])
            .deny_reason(&WriteTargets::none())
            .is_none());
    }

    #[test]
    fn an_unbounded_write_is_denied_by_any_non_empty_scope() {
        assert_eq!(
            scope(&["orders"]).deny_reason(&WriteTargets::Unbounded),
            Some(OutOfScope::Unbounded)
        );
    }

    #[test]
    fn inserting_into_an_unbounded_target_set_changes_nothing() {
        let mut targets = WriteTargets::Unbounded;
        targets.insert("orders");
        assert_eq!(targets, WriteTargets::Unbounded);
    }

    #[test]
    fn protection_defaults_to_off_and_stays_off_without_input() {
        let protection = BaselineProtection::configure(false, &[] as &[&str]).unwrap();
        assert_eq!(protection, BaselineProtection::Off);
        assert!(!protection.protects(&WriteTargets::of("orders")));
        assert!(!protection.protects(&WriteTargets::Unbounded));
    }

    #[test]
    fn the_blanket_flag_protects_every_namespace() {
        let protection = BaselineProtection::configure(true, &[] as &[&str]).unwrap();
        assert_eq!(protection, BaselineProtection::All);
        assert!(protection.protects(&WriteTargets::of("anything")));
        assert!(
            protection.protects(&WriteTargets::none()),
            "a write that names nothing must not slip past blanket protection"
        );
    }

    #[test]
    fn named_namespaces_narrow_the_blanket_flag() {
        let protection = BaselineProtection::configure(true, &["orders", "shop-*"]).unwrap();
        assert!(protection.protects(&WriteTargets::of("orders")));
        assert!(protection.protects(&WriteTargets::of("shop-checkout")));
        assert!(!protection.protects(&WriteTargets::of("sandbox")));
        assert!(
            protection.protects(&WriteTargets::Unbounded),
            "an unbounded write could land in a protected namespace"
        );
    }

    #[test]
    fn a_mixed_write_is_protected_when_any_namespace_is() {
        let protection = BaselineProtection::configure(false, &["orders"]).unwrap();
        let mut targets = WriteTargets::of("sandbox");
        targets.insert("orders");
        assert!(protection.protects(&targets));
    }

    #[test]
    fn a_bad_pattern_fails_configuration_rather_than_being_dropped() {
        assert!(BaselineProtection::configure(false, &["a/b"]).is_err());
        assert!(NamespaceScope::parse(&["a b"]).is_err());
    }
}
