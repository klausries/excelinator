//! xl-serial: spreadsheet date serial numbers.
//!
//! Two date systems exist in SpreadsheetML (`workbookPr/@date1904`):
//! - 1900 system: serial 1 is 1900-01-01. Serial 60 is 1900-02-29, a day that never
//!   existed. Excel keeps it for compatibility with Lotus 1-2-3, so every serial
//!   from 61 on is one higher than the true day count. Serial 0 displays as
//!   1900-01-00.
//! - 1904 system: serial 0 is 1904-01-01. No fictitious day.
//!
//! The last valid date in both systems is 9999-12-31.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DateSystem {
    #[default]
    V1900,
    V1904,
}

/// Largest serial in the 1900 system (9999-12-31).
pub const MAX_SERIAL_1900: i64 = 2_958_465;
/// Largest serial in the 1904 system (9999-12-31).
pub const MAX_SERIAL_1904: i64 = 2_957_003;

/// Days since 1970-01-01 in the proleptic Gregorian calendar.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { yoe + era * 400 + 1 } else { yoe + era * 400 }, m, d)
}

fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

fn days_in_month(y: i64, m: i64) -> i64 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ => 28 + is_leap(y) as i64,
    }
}

const EPOCH_1900: i64 = -25_569; // 1899-12-30 as days since 1970-01-01
const EPOCH_1904: i64 = -24_107; // 1904-01-01

/// Serial number of a calendar date, or `None` if the date is outside the system.
/// In the 1900 system `(1900, 2, 29)` is accepted and yields 60, `(1900, 1, 0)` yields 0.
pub fn serial_from_ymd(year: i32, month: u32, day: u32, system: DateSystem) -> Option<i64> {
    let (y, m, d) = (year as i64, month as i64, day as i64);
    if !(1..=12).contains(&m) || y > 9999 {
        return None;
    }
    match system {
        DateSystem::V1900 => {
            if (y, m, d) == (1900, 1, 0) {
                return Some(0);
            }
            if (y, m, d) == (1900, 2, 29) {
                return Some(60);
            }
            if y < 1900 || d < 1 || d > days_in_month(y, m) {
                return None;
            }
            let s = days_from_civil(y, m, d) - EPOCH_1900;
            Some(if s < 61 { s - 1 } else { s })
        }
        DateSystem::V1904 => {
            if y < 1904 || d < 1 || d > days_in_month(y, m) {
                return None;
            }
            Some(days_from_civil(y, m, d) - EPOCH_1904)
        }
    }
}

/// Calendar date of a serial number as (year, month, day), or `None` if out of range.
/// In the 1900 system serial 60 yields `(1900, 2, 29)` and serial 0 yields `(1900, 1, 0)`.
pub fn ymd_from_serial(serial: i64, system: DateSystem) -> Option<(i32, u32, u32)> {
    let days = match system {
        DateSystem::V1900 => {
            if !(0..=MAX_SERIAL_1900).contains(&serial) {
                return None;
            }
            match serial {
                0 => return Some((1900, 1, 0)),
                60 => return Some((1900, 2, 29)),
                s if s < 60 => s + 1 + EPOCH_1900,
                s => s + EPOCH_1900,
            }
        }
        DateSystem::V1904 => {
            if !(0..=MAX_SERIAL_1904).contains(&serial) {
                return None;
            }
            serial + EPOCH_1904
        }
    };
    let (y, m, d) = civil_from_days(days);
    Some((y as i32, m as u32, d as u32))
}

/// Day of week as Excel's `WEEKDAY(serial, 1)`: 1 = Sunday ... 7 = Saturday.
/// In the 1900 system this follows the serial, so dates before 1900-03-01 are one
/// weekday off from the real calendar, exactly as in Excel.
pub fn weekday(serial: i64, system: DateSystem) -> Option<u32> {
    let offset = match system {
        DateSystem::V1900 => {
            if !(0..=MAX_SERIAL_1900).contains(&serial) {
                return None;
            }
            serial + 6
        }
        DateSystem::V1904 => {
            if !(0..=MAX_SERIAL_1904).contains(&serial) {
                return None;
            }
            serial + 5 // 1904-01-01 was a Friday
        }
    };
    Some((offset.rem_euclid(7) + 1) as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use DateSystem::*;

    #[test]
    fn known_values_1900() {
        for (s, ymd) in [
            (0, (1900, 1, 0)),
            (1, (1900, 1, 1)),
            (59, (1900, 2, 28)),
            (60, (1900, 2, 29)),
            (61, (1900, 3, 1)),
            (366, (1900, 12, 31)),
            (367, (1901, 1, 1)),
            (25_569, (1970, 1, 1)),
            (36_526, (2000, 1, 1)),
            (36_585, (2000, 2, 29)),
            (45_000, (2023, 3, 15)),
            (MAX_SERIAL_1900, (9999, 12, 31)),
        ] {
            assert_eq!(ymd_from_serial(s, V1900), Some(ymd), "serial {s}");
            assert_eq!(serial_from_ymd(ymd.0, ymd.1, ymd.2, V1900), Some(s), "{ymd:?}");
        }
    }

    #[test]
    fn known_values_1904() {
        for (s, ymd) in [(0, (1904, 1, 1)), (59, (1904, 2, 29)), (24_107, (1970, 1, 1)), (MAX_SERIAL_1904, (9999, 12, 31))] {
            assert_eq!(ymd_from_serial(s, V1904), Some(ymd));
            assert_eq!(serial_from_ymd(ymd.0, ymd.1, ymd.2, V1904), Some(s));
        }
    }

    #[test]
    fn systems_differ_by_1462_days() {
        for (y, m, d) in [(1904, 1, 1), (1970, 1, 1), (2024, 2, 29), (9999, 12, 31)] {
            assert_eq!(serial_from_ymd(y, m, d, V1900).unwrap() - serial_from_ymd(y, m, d, V1904).unwrap(), 1462);
        }
    }

    #[test]
    fn every_serial_round_trips_and_dates_ascend() {
        for (system, max) in [(V1900, MAX_SERIAL_1900), (V1904, MAX_SERIAL_1904)] {
            let mut prev = (0, 0, 0);
            for s in 0..=max {
                let ymd = ymd_from_serial(s, system).unwrap();
                assert_eq!(serial_from_ymd(ymd.0, ymd.1, ymd.2, system), Some(s));
                assert!(ymd > prev, "serial {s}: {ymd:?} after {prev:?}");
                prev = ymd;
            }
        }
    }

    #[test]
    fn out_of_range() {
        assert_eq!(ymd_from_serial(-1, V1900), None);
        assert_eq!(ymd_from_serial(MAX_SERIAL_1900 + 1, V1900), None);
        assert_eq!(serial_from_ymd(1899, 12, 31, V1900), None);
        assert_eq!(serial_from_ymd(1901, 2, 29, V1900), None);
        assert_eq!(serial_from_ymd(2023, 13, 1, V1900), None);
        assert_eq!(serial_from_ymd(1903, 12, 31, V1904), None);
    }

    #[test]
    fn weekdays() {
        assert_eq!(weekday(1, V1900), Some(1)); // Excel says Sunday; it really was a Monday
        assert_eq!(weekday(61, V1900), Some(5)); // 1900-03-01, Thursday
        assert_eq!(weekday(45_000, V1900), Some(4)); // 2023-03-15, Wednesday
        assert_eq!(weekday(0, V1904), Some(6)); // 1904-01-01, Friday
        assert_eq!(weekday(43_538, V1904), weekday(45_000, V1900));
    }
}
