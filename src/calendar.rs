//! Pure calendar math for the menubar clock popover: month grids, weekday of a
//! date, and month stepping. No chrono dependency — civil-date arithmetic only
//! (days-from-civil per Howard Hinnant's algorithms), so it is fully testable.

/// Days in `month` (1-12) of `year`, honouring leap years.
pub fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_leap(year) {
                29
            } else {
                28
            }
        }
        _ => 30,
    }
}

/// Whether `year` is a Gregorian leap year.
pub fn is_leap(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

/// Days since the civil epoch 1970-01-01 for a (year, month 1-12, day 1-31).
fn days_from_civil(y: i32, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y } as i64;
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400; // [0, 399]
    let mp = ((m as i64) + 9) % 12; // [0, 11], Mar = 0
    let doy = (153 * mp + 2) / 5 + (d as i64) - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146097 + doe - 719468
}

/// Weekday of a date, Monday = 0 … Sunday = 6.
pub fn weekday(year: i32, month: u32, day: u32) -> u32 {
    // 1970-01-01 was a Thursday (Mon=0 → 3).
    (days_from_civil(year, month, day).rem_euclid(7) as u32 + 3) % 7
}

/// Step (year, month) by `delta` months (delta may be negative).
pub fn add_months(year: i32, month: u32, delta: i32) -> (i32, u32) {
    let total = year as i64 * 12 + (month as i64 - 1) + delta as i64;
    (total.div_euclid(12) as i32, (total.rem_euclid(12) + 1) as u32)
}

/// English month name for `month` (1-12).
pub fn month_name(month: u32) -> &'static str {
    const NAMES: [&str; 12] = [
        "January", "February", "March", "April", "May", "June", "July", "August", "September",
        "October", "November", "December",
    ];
    NAMES[((month.clamp(1, 12)) - 1) as usize]
}

/// The weeks of a month as rows of `Option<day>`, Monday-first. Leading/trailing
/// `None`s pad the first/last week so every row has exactly 7 entries.
pub fn month_grid(year: i32, month: u32) -> Vec<[Option<u32>; 7]> {
    let first_wd = weekday(year, month, 1) as usize;
    let n = days_in_month(year, month);
    let mut weeks = Vec::new();
    let mut row = [None; 7];
    let mut col = first_wd;
    for d in 1..=n {
        row[col] = Some(d);
        col += 1;
        if col == 7 {
            weeks.push(row);
            row = [None; 7];
            col = 0;
        }
    }
    if col != 0 {
        weeks.push(row);
    }
    weeks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leap_years() {
        assert!(is_leap(2024));
        assert!(!is_leap(2026));
        assert!(!is_leap(1900));
        assert!(is_leap(2000));
        assert_eq!(days_in_month(2024, 2), 29);
        assert_eq!(days_in_month(2026, 2), 28);
        assert_eq!(days_in_month(2026, 6), 30);
    }

    #[test]
    fn known_weekdays() {
        assert_eq!(weekday(1970, 1, 1), 3); // Thursday
        assert_eq!(weekday(2026, 6, 10), 2); // Wednesday
        assert_eq!(weekday(2000, 1, 1), 5); // Saturday
        assert_eq!(weekday(2026, 6, 1), 0); // Monday
    }

    #[test]
    fn month_stepping_wraps_years() {
        assert_eq!(add_months(2026, 6, 1), (2026, 7));
        assert_eq!(add_months(2026, 12, 1), (2027, 1));
        assert_eq!(add_months(2026, 1, -1), (2025, 12));
        assert_eq!(add_months(2026, 6, -18), (2024, 12));
    }

    #[test]
    fn june_2026_grid_shape() {
        let g = month_grid(2026, 6);
        // June 2026 starts on a Monday and has 30 days → 5 rows, no leading pad.
        assert_eq!(g.len(), 5);
        assert_eq!(g[0][0], Some(1));
        assert_eq!(g[4][1], Some(30));
        assert_eq!(g[4][2], None);
        for row in &g {
            assert_eq!(row.len(), 7);
        }
    }

    #[test]
    fn padded_first_week() {
        // Feb 2026 starts on a Sunday → six leading Nones.
        let g = month_grid(2026, 2);
        assert_eq!(&g[0][..6], &[None; 6]);
        assert_eq!(g[0][6], Some(1));
        assert_eq!(days_in_month(2026, 2), 28);
    }
}
