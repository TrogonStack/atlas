use trogon_atlas_core::authored_date::AuthoredDate;

#[test]
fn authored_dates_preserve_calendar_values_without_inventing_instants() {
    for value in [
        "0001-01-01",
        "1900-02-28",
        "2000-02-29",
        "2024-02-29",
        "2026-10-28",
        "9999-12-31",
    ] {
        let date = value.parse::<AuthoredDate>().expect("valid calendar date");
        assert_eq!(date.as_str(), value);
        assert_eq!(date.to_string(), value);
    }
}

#[test]
fn authored_dates_reject_invalid_days_and_noncanonical_dates() {
    for value in [
        "",
        "0000-01-01",
        "1900-02-29",
        "2023-02-29",
        "2024-02-30",
        "2026-04-31",
        "2026-00-01",
        "2026-13-01",
        "2026-01-00",
        "2026-01-32",
        "2026-1-01",
        "2026-01-1",
        "26-01-01",
        "+2026-01-01",
        "-0001-01-01",
        "10000-01-01",
        "2026/01/01",
        " 2026-01-01",
        "2026-01-01\n",
        "2026-01-01T00:00:00Z",
        "２０２６-01-01",
    ] {
        assert!(value.parse::<AuthoredDate>().is_err(), "accepted {value:?}");
    }
}

#[test]
fn authored_date_order_is_chronological_across_months_and_years() {
    let before = "2024-02-29".parse::<AuthoredDate>().expect("valid date");
    let next_month = "2024-03-01".parse::<AuthoredDate>().expect("valid date");
    let next_year = "2025-01-01".parse::<AuthoredDate>().expect("valid date");
    assert!(before < next_month);
    assert!(next_month < next_year);
    assert_eq!(before.clone(), before);
}
