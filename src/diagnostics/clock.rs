//! UTC time as text, without a date library: log lines and crash reports only need it for reading.

use std::time::{SystemTime, UNIX_EPOCH};

/// Days since 1970-01-01 -> (year, month, day), proleptic Gregorian (Howard Hinnant's algorithm).
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn parts(t: SystemTime) -> (i64, u32, u32, u32, u32, u32) {
    let secs = t.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs()) as i64;
    let (y, m, d) = civil(secs.div_euclid(86_400));
    let rem = secs.rem_euclid(86_400);
    (
        y,
        m,
        d,
        (rem / 3600) as u32,
        (rem % 3600 / 60) as u32,
        (rem % 60) as u32,
    )
}

/// `2026-11-14T14:50:00Z`
pub fn iso(t: SystemTime) -> String {
    let (y, mo, d, h, mi, s) = parts(t);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
}

/// `20261114-145000`, for file names.
pub fn stamp(t: SystemTime) -> String {
    let (y, mo, d, h, mi, s) = parts(t);
    format!("{y:04}{mo:02}{d:02}-{h:02}{mi:02}{s:02}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn known_instants() {
        assert_eq!(iso(UNIX_EPOCH), "1970-01-01T00:00:00Z");
        // 2024-02-29 12:34:56 UTC (a leap day)
        assert_eq!(
            iso(UNIX_EPOCH + Duration::from_secs(1_709_210_096)),
            "2024-02-29T12:34:56Z"
        );
        assert_eq!(
            stamp(UNIX_EPOCH + Duration::from_secs(1_709_210_096)),
            "20240229-123456"
        );
        // 2000-03-01 00:00:00 UTC
        assert_eq!(
            iso(UNIX_EPOCH + Duration::from_secs(951_868_800)),
            "2000-03-01T00:00:00Z"
        );
    }
}
