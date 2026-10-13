//! The times the hosting platforms write, as Unix seconds.

use time::format_description::well_known::Rfc3339;
use time::macros::format_description;
use time::{OffsetDateTime, PrimitiveDateTime, UtcOffset};

/// An RFC 3339 time, in UTC or with an offset, with or without a fraction of a second
/// (`2024-05-01T12:00:00Z`, `2024-05-01T12:00:00.1234567Z`, `2024-05-01T05:00:00-07:00`).
pub(crate) fn parse_rfc3339(text: &str) -> Option<i64> {
    OffsetDateTime::parse(text, &Rfc3339)
        .ok()
        .map(OffsetDateTime::unix_timestamp)
}

/// A time as GitHub's token header writes it: `2026-11-01 12:00:00 UTC`, or with an offset such
/// as `+0200` in place of `UTC`.
pub(crate) fn parse_token_expiry(text: &str) -> Option<i64> {
    let mut parts = text.split_whitespace();
    let (date, clock, zone) = (parts.next()?, parts.next()?, parts.next()?);
    let local = PrimitiveDateTime::parse(
        &format!("{date} {clock}"),
        format_description!("[year]-[month]-[day] [hour]:[minute]:[second]"),
    )
    .ok()?;
    let offset = match zone {
        "UTC" | "Z" => UtcOffset::UTC,
        _ => UtcOffset::parse(zone, format_description!("[offset_hour sign:mandatory][offset_minute]")).ok()?,
    };
    Some(local.assume_offset(offset).unix_timestamp())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc3339_times_are_unix_seconds_with_fractions_and_offsets() {
        assert_eq!(parse_rfc3339("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_rfc3339("2024-05-01T12:00:00Z"), Some(1_714_564_800));
        assert_eq!(parse_rfc3339("2024-05-01T12:00:00.1234567Z"), Some(1_714_564_800));
        assert_eq!(parse_rfc3339("2024-05-01T05:00:00.000-07:00"), Some(1_714_564_800));
        assert_eq!(parse_rfc3339("2024-05-01T14:00:00+02:00"), Some(1_714_564_800));
        // A leap day, and the day after it.
        assert_eq!(parse_rfc3339("2024-02-29T23:59:59Z"), Some(1_709_251_199));
        assert_eq!(parse_rfc3339("2024-03-01T00:00:00Z"), Some(1_709_251_200));
        assert_eq!(parse_rfc3339("1969-12-31T23:59:59Z"), Some(-1));
        for bad in ["", "not a time", "2024-05-01", "2024-13-01T00:00:00Z", "x-y-zT0:0:0Z"] {
            assert_eq!(parse_rfc3339(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn token_expiries_are_read_in_utc_and_with_offsets() {
        assert_eq!(parse_token_expiry("2026-11-01 12:00:00 UTC"), Some(1_793_534_400));
        assert_eq!(parse_token_expiry("2024-02-29 10:30:00 +0200"), Some(1_709_195_400));
        assert_eq!(parse_token_expiry("2024-02-29 01:30:00 -0700"), Some(1_709_195_400));
        assert_eq!(parse_token_expiry("tomorrow"), None);
        assert_eq!(parse_token_expiry("2026-13-01 12:00:00 UTC"), None);
    }
}
