//! Shared rule for "what's a safe namespace / slug component". Used at
//! the gRPC boundary (`trogon-atlas-server/conv.rs`), the git mirror
//! (`trogon-atlas-server/git_mirror.rs`), and the NATS subject builder
//! (`trogon-atlas-store/nats.rs`). Co-locating the predicate prevents a
//! future relaxation on one side from inadvertently bypassing the others.
//!
//! Allowed characters: ASCII alphanumerics plus `-`, `_`, `.`. Reject:
//! empty, `.` / `..`, leading `.`, control characters, and anything over
//! 200 chars. The dot is allowed in the middle so existing fixtures like
//! `order.placed` keep working.

/// Reason a value failed validation. Carries `&'static str` and the
/// offending value so callers can lift it into their own error type with
/// a useful message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdComponentError {
    Empty,
    TooLong,
    DotOrDotDot,
    LeadingDot,
    InvalidChar(char),
}

impl std::fmt::Display for IdComponentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IdComponentError::Empty => f.write_str("must not be empty"),
            IdComponentError::TooLong => f.write_str("exceeds 200 chars"),
            IdComponentError::DotOrDotDot => f.write_str("must not be '.' or '..'"),
            IdComponentError::LeadingDot => f.write_str("must not start with '.'"),
            IdComponentError::InvalidChar(ch) => write!(
                f,
                "contains invalid character {ch:?}; allowed: [A-Za-z0-9_.-]"
            ),
        }
    }
}

impl std::error::Error for IdComponentError {}

/// Returns `Ok(())` if `value` is a safe id component, otherwise a tagged
/// error describing what's wrong. The caller is responsible for surfacing
/// the error in its native type (`Status::invalid_argument`, `anyhow`, …).
pub fn validate_id_component(value: &str) -> Result<(), IdComponentError> {
    if value.is_empty() {
        return Err(IdComponentError::Empty);
    }
    if value.len() > 200 {
        return Err(IdComponentError::TooLong);
    }
    if value == "." || value == ".." {
        return Err(IdComponentError::DotOrDotDot);
    }
    if value.starts_with('.') {
        return Err(IdComponentError::LeadingDot);
    }
    for ch in value.chars() {
        let safe = ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.');
        if !safe {
            return Err(IdComponentError::InvalidChar(ch));
        }
    }
    Ok(())
}

/// Cheap boolean check for hot paths that only need a yes/no answer.
#[must_use]
pub fn is_safe_id_component(value: &str) -> bool {
    validate_id_component(value).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_typical_components() {
        for v in [
            "orders",
            "ecommerce",
            "order.placed",
            "order-placed",
            "v2",
            "a_b",
        ] {
            assert!(validate_id_component(v).is_ok(), "rejected {v:?}");
        }
    }

    #[test]
    fn rejects_path_traversal() {
        for v in ["..", ".", "../etc", "./foo", "/abs", "a/b", "a\\b"] {
            assert!(validate_id_component(v).is_err(), "accepted hostile {v:?}");
        }
    }

    #[test]
    fn rejects_leading_dot() {
        for v in [".git", ".hidden", "..rc"] {
            assert!(
                validate_id_component(v).is_err(),
                "accepted leading-dot {v:?}"
            );
        }
    }

    #[test]
    fn rejects_control_chars() {
        for v in ["a\nb", "a\tb", "a b", "a\u{202E}b"] {
            assert!(validate_id_component(v).is_err(), "accepted hostile {v:?}");
        }
    }

    #[test]
    fn rejects_overlong() {
        let long = "a".repeat(201);
        assert!(validate_id_component(&long).is_err());
    }
}

#[cfg(test)]
mod property_tests {
    use proptest::prelude::*;

    use super::*;

    // Strategy: non-empty string of safe chars, optionally with dots in the middle.
    // Constructed so it can never be "." or ".." and never starts with '.'.
    fn safe_component() -> impl Strategy<Value = String> {
        // Start with one alphanumeric char, then zero or more safe chars (including dot).
        let mid_chars = "[a-zA-Z0-9][a-zA-Z0-9_.-]{0,50}";
        mid_chars.prop_map(|s| s)
    }

    // Strategy: string containing at least one character outside the allowed set.
    fn string_with_invalid_char() -> impl Strategy<Value = String> {
        // Pick a definitely-invalid character: space, slash, colon, newline, etc.
        let invalid_chars: Vec<char> = vec![' ', '/', '\\', ':', '@', '!', '?', '\n', '\t', '\0'];
        (
            "[a-zA-Z0-9]{1,10}".prop_map(|s| s),
            prop::sample::select(invalid_chars),
            "[a-zA-Z0-9]{0,10}".prop_map(|s| s),
        )
            .prop_map(|(prefix, bad, suffix)| format!("{prefix}{bad}{suffix}"))
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        #[test]
        fn valid_by_construction_always_pass(s in safe_component()) {
            // All strings produced by safe_component() must pass validation.
            // If proptest finds a counterexample it will print it.
            prop_assert!(
                validate_id_component(&s).is_ok(),
                "safe component rejected: {:?}",
                s
            );
        }

        #[test]
        fn strings_with_invalid_chars_always_fail(s in string_with_invalid_char()) {
            // Every string containing a character outside [A-Za-z0-9_.-] must fail.
            prop_assert!(
                validate_id_component(&s).is_err(),
                "invalid-char string accepted: {:?}",
                s
            );
        }

        #[test]
        fn empty_string_always_rejected(_: ()) {
            prop_assert!(validate_id_component("").is_err());
        }

        #[test]
        fn exactly_201_chars_always_rejected(c in "[a-zA-Z0-9]") {
            let long = c.repeat(201);
            prop_assert!(
                validate_id_component(&long).is_err(),
                "201-char string was accepted"
            );
        }

        #[test]
        fn exactly_200_chars_always_accepted(c in "[a-zA-Z0-9]") {
            let at_limit = c.repeat(200);
            prop_assert!(
                validate_id_component(&at_limit).is_ok(),
                "200-char string was rejected"
            );
        }

        #[test]
        fn strings_starting_with_dot_are_rejected(tail in "[a-zA-Z0-9_.-]{1,50}") {
            let s = format!(".{tail}");
            prop_assert!(
                validate_id_component(&s).is_err(),
                "leading-dot string accepted: {:?}",
                s
            );
        }
    }
}
