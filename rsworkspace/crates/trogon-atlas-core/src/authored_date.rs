use std::{fmt, str::FromStr};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AuthoredDate(String);

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("expected a Gregorian calendar date in YYYY-MM-DD format")]
pub struct InvalidAuthoredDate;

impl FromStr for AuthoredDate {
    type Err = InvalidAuthoredDate;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.len() != 10
            || value.starts_with("0000")
            || !value.bytes().enumerate().all(|(index, byte)| {
                if index == 4 || index == 7 {
                    byte == b'-'
                } else {
                    byte.is_ascii_digit()
                }
            })
        {
            return Err(InvalidAuthoredDate);
        }
        chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d").map_err(|_| InvalidAuthoredDate)?;
        Ok(Self(value.to_owned()))
    }
}

impl AuthoredDate {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for AuthoredDate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}
