use std::{borrow::Cow, fmt::Write};

use time::{format_description::well_known::Rfc3339, OffsetDateTime, UtcOffset};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimestampPrecision {
    Seconds,
    Milliseconds,
    Nanoseconds,
}

/// Format UTC with the fixed precision used by Atlas's stored timestamps.
#[must_use]
pub fn format_utc(at: OffsetDateTime, precision: TimestampPrecision) -> String {
    let at = at.to_offset(UtcOffset::UTC);
    let (year, month, day) = at.to_calendar_date();
    let year_sign = match year {
        ..0 => "-",
        10_000.. => "+",
        _ => "",
    };
    let year = year.unsigned_abs();
    let month = u8::from(month);
    let (hour, minute, second, nanosecond) = at.to_hms_nano();
    let mut result =
        format!("{year_sign}{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}");
    match precision {
        TimestampPrecision::Seconds => {}
        TimestampPrecision::Milliseconds => {
            let _ = write!(result, ".{:03}", nanosecond / 1_000_000);
        }
        TimestampPrecision::Nanoseconds => {
            let _ = write!(result, ".{nanosecond:09}");
        }
    }
    result.push('Z');
    result
}

/// Keep the inserted leap second distinct for ordering while preserving its Unix projection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rfc3339Timestamp {
    seconds: i64,
    nanoseconds: u32,
}

impl Rfc3339Timestamp {
    #[must_use]
    pub fn is_after(self, now: OffsetDateTime) -> bool {
        (self.seconds, self.nanoseconds) > (now.unix_timestamp(), now.nanosecond())
    }

    #[must_use]
    pub fn unix_millis(self) -> i64 {
        self.seconds * 1_000 + i64::from(self.nanoseconds / 1_000_000)
    }

    #[must_use]
    pub fn unix_micros(self) -> i64 {
        self.seconds * 1_000_000 + i64::from(self.nanoseconds / 1_000)
    }
}

/// Read the timestamp spellings already accepted at Atlas's wire and storage boundaries.
#[must_use]
pub fn parse_rfc3339(value: &str) -> Option<Rfc3339Timestamp> {
    if !matches!(value.as_bytes().get(10), Some(b'T' | b't' | b' ')) {
        return None;
    }

    let mut normalized = Cow::Borrowed(value);
    if let Some((timestamp, offset)) = value.rsplit_once('\u{2212}') {
        if timestamp.len() >= 19 {
            normalized = Cow::Owned(format!("{timestamp}-{offset}"));
        }
    }

    // Atlas accepted explicit leap seconds without consulting a leap-second table.
    let leap_second = normalized.get(17..19) == Some("60");
    if leap_second {
        normalized.to_mut().replace_range(17..19, "59");
    }

    let at = OffsetDateTime::parse(&normalized, &Rfc3339).ok()?;
    Some(Rfc3339Timestamp {
        seconds: at.unix_timestamp(),
        nanoseconds: at.nanosecond() + if leap_second { 1_000_000_000 } else { 0 },
    })
}
