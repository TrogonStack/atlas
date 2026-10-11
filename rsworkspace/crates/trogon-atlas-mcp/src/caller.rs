use std::fmt;

use rmcp::{service::RequestContext, ErrorData, RoleServer};

pub const BRANCH_HEADER: &str = "x-trogon-atlas-branch";

/// Who an MCP request acts as, and on which branch, read from that request's
/// own headers so concurrent callers never borrow each other's identity.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Caller {
    pub token: Option<CallerToken>,
    pub branch: Option<String>,
}

impl Caller {
    /// Requests that did not arrive over HTTP carry no headers at all, so
    /// they act anonymously against baseline.
    pub fn from_context(context: &RequestContext<RoleServer>) -> Result<Self, ErrorData> {
        match context.extensions.get::<http::request::Parts>() {
            Some(parts) => Self::from_headers(&parts.headers),
            None => Ok(Self::default()),
        }
    }

    pub fn from_headers(headers: &http::HeaderMap) -> Result<Self, ErrorData> {
        let branch = match headers.get(BRANCH_HEADER) {
            None => None,
            Some(value) => {
                let value = value.to_str().map_err(|_| {
                    ErrorData::invalid_request(format!("{BRANCH_HEADER} must be ASCII"), None)
                })?;
                Some(value.trim().to_owned()).filter(|branch| !branch.is_empty())
            }
        };
        Ok(Self {
            token: CallerToken::from_headers(headers)?,
            branch,
        })
    }
}

/// The bearer token an MCP request arrived with. It is forwarded verbatim to
/// trogon-atlas-server, so every caller is authenticated and authorized as
/// itself.
#[derive(Clone, PartialEq, Eq)]
pub struct CallerToken(String);

impl fmt::Debug for CallerToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CallerToken(<redacted>)")
    }
}

impl CallerToken {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// `None` when the request carries no `Authorization` header, which an
    /// anonymous server accepts and an authenticating one rejects.
    pub fn from_headers(headers: &http::HeaderMap) -> Result<Option<Self>, ErrorData> {
        let Some(value) = headers.get(http::header::AUTHORIZATION) else {
            return Ok(None);
        };
        let malformed = || {
            ErrorData::invalid_request("the Authorization header must be `Bearer <token>`", None)
        };
        let value = value.to_str().map_err(|_| malformed())?;
        let (scheme, token) = value.split_once(' ').ok_or_else(malformed)?;
        let token = token.trim();
        if !scheme.eq_ignore_ascii_case("bearer") || token.is_empty() {
            return Err(malformed());
        }
        Ok(Some(Self(token.to_owned())))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn headers(authorization: &str) -> http::HeaderMap {
        let mut headers = http::HeaderMap::new();
        headers.insert(http::header::AUTHORIZATION, authorization.parse().unwrap());
        headers
    }

    #[test]
    fn absent_header_is_anonymous() {
        assert_eq!(
            CallerToken::from_headers(&http::HeaderMap::new()).unwrap(),
            None
        );
    }

    #[test]
    fn bearer_scheme_is_case_insensitive() {
        let token = CallerToken::from_headers(&headers("bearer abc"))
            .unwrap()
            .unwrap();
        assert_eq!(token.as_str(), "abc");
        let token = CallerToken::from_headers(&headers("Bearer abc"))
            .unwrap()
            .unwrap();
        assert_eq!(token.as_str(), "abc");
    }

    #[test]
    fn other_schemes_and_empty_tokens_are_rejected() {
        for value in ["Basic dXNlcjpwYXNz", "Bearer", "Bearer   ", "abc"] {
            assert!(
                CallerToken::from_headers(&headers(value)).is_err(),
                "{value:?} should be rejected"
            );
        }
    }

    #[test]
    fn branch_header_scopes_the_caller() {
        let mut headers = headers("Bearer abc");
        headers.insert(BRANCH_HEADER, "alex/rework".parse().unwrap());
        let caller = Caller::from_headers(&headers).unwrap();
        assert_eq!(caller.branch.as_deref(), Some("alex/rework"));
        assert_eq!(caller.token.unwrap().as_str(), "abc");
        assert_eq!(
            Caller::from_headers(&http::HeaderMap::new()).unwrap(),
            Caller::default()
        );
    }

    #[test]
    fn debug_never_prints_the_token() {
        let token = CallerToken::from_headers(&headers("Bearer secret"))
            .unwrap()
            .unwrap();
        assert!(!format!("{token:?}").contains("secret"));
    }
}
