#![allow(clippy::unwrap_used)]

use time::{format_description::well_known::Rfc3339, OffsetDateTime};
use trogon_atlas_core::timestamp::{format_utc, parse_rfc3339, TimestampPrecision};

#[test]
fn utc_output_keeps_the_requested_precision() {
    let instant = OffsetDateTime::parse("2026-11-18T22:01:02.010000009+03:00", &Rfc3339).unwrap();
    assert_eq!(
        format_utc(instant, TimestampPrecision::Seconds),
        "2026-11-18T19:01:02Z"
    );
    assert_eq!(
        format_utc(instant, TimestampPrecision::Milliseconds),
        "2026-11-18T19:01:02.010Z"
    );
    assert_eq!(
        format_utc(instant, TimestampPrecision::Nanoseconds),
        "2026-11-18T19:01:02.010000009Z"
    );

    let whole_second = OffsetDateTime::parse("2026-11-18T19:01:02Z", &Rfc3339).unwrap();
    assert_eq!(
        format_utc(whole_second, TimestampPrecision::Milliseconds),
        "2026-11-18T19:01:02.000Z"
    );
}

#[test]
fn parsing_preserves_supported_spelling_and_flooring() {
    let canonical = parse_rfc3339("1969-12-31T23:59:59.999999999Z").unwrap();
    assert_eq!(canonical.unix_millis(), -1);
    assert_eq!(canonical.unix_micros(), -1);
    for value in [
        "1969-12-31t23:59:59.999999999999z",
        "1969-12-31 18:59:59.999999999\u{2212}05:00",
        "1970-01-01T00:29:59.999999999+00:30",
    ] {
        assert_eq!(parse_rfc3339(value), Some(canonical));
    }

    for value in [
        "1969-12-31/23:59:59Z",
        "1969-12-31T23:59:61Z",
        "1969-12-31T23:59:59+24:00",
        "1969-12-31T23:59:59Z ",
        "2026\u{2212}11-18T19:01:02Z",
    ] {
        assert!(parse_rfc3339(value).is_none(), "accepted {value}");
    }
}

#[test]
fn leap_seconds_keep_order_and_unix_projection() {
    let leap_second = parse_rfc3339("2016-12-31T18:59:60.1234567899-05:00").unwrap();
    let preceding = OffsetDateTime::parse("2016-12-31T23:59:59.999999999Z", &Rfc3339).unwrap();
    let following = OffsetDateTime::parse("2017-01-01T00:00:00Z", &Rfc3339).unwrap();

    assert!(leap_second.is_after(preceding));
    assert!(!leap_second.is_after(following));
    assert_eq!(leap_second.unix_millis(), 1_483_228_800_123);
    assert_eq!(leap_second.unix_micros(), 1_483_228_800_123_456);
    assert!(parse_rfc3339("2026-11-18T19:01:60Z").is_some());
}
