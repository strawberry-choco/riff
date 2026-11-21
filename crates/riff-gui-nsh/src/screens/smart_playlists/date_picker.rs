//! The date and duration value helpers for the rule editor.
//!
//! nsh 0.6.1 ships a `DatePicker` component, so the plan's custom popover is
//! not needed: a rule's date is typed as `YYYY-MM-DD HH:MM` and checked
//! here, with a Today shortcut the editor offers. The parse is strict about
//! shape and range so a stored rule always re-reads as the same value.

use riff_backend::domain::CivilDateTime;

/// The one refusal every unreadable value carries (the egui editor's
/// sentence).
const UNREADABLE: &str = "this rule's value can't be read as this field's kind";

/// A `YYYY-MM-DD[ HH:MM[:SS]]` wall-clock value.
pub fn parse_civil(text: &str) -> Result<CivilDateTime, String> {
    let text = text.trim();
    let (date, time) = text.split_once(' ').unwrap_or((text, ""));
    let mut date_parts = date.split('-');
    let year: i32 = date_parts
        .next()
        .ok_or_else(|| UNREADABLE.to_owned())?
        .parse()
        .map_err(|_| UNREADABLE.to_owned())?;
    let month: u8 = date_parts
        .next()
        .ok_or_else(|| UNREADABLE.to_owned())?
        .parse()
        .map_err(|_| UNREADABLE.to_owned())?;
    let day: u8 = date_parts
        .next()
        .ok_or_else(|| UNREADABLE.to_owned())?
        .parse()
        .map_err(|_| UNREADABLE.to_owned())?;
    if date_parts.next().is_some()
        || !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || day > days_in_month(year, month)
    {
        return Err(UNREADABLE.to_owned());
    }
    let (hour, minute, second) = if time.is_empty() {
        (0, 0, 0)
    } else {
        let mut time_parts = time.split(':');
        let hour: u8 = time_parts
            .next()
            .ok_or_else(|| UNREADABLE.to_owned())?
            .parse()
            .map_err(|_| UNREADABLE.to_owned())?;
        let minute: u8 = time_parts
            .next()
            .ok_or_else(|| UNREADABLE.to_owned())?
            .parse()
            .map_err(|_| UNREADABLE.to_owned())?;
        let second: u8 = time_parts
            .next()
            .map_or(Ok(0), str::parse)
            .map_err(|_| UNREADABLE.to_owned())?;
        if time_parts.next().is_some() || hour > 23 || minute > 59 || second > 59 {
            return Err(UNREADABLE.to_owned());
        }
        (hour, minute, second)
    };
    Ok(CivilDateTime {
        year,
        month,
        day,
        hour,
        minute,
        second,
    })
}

/// The stored form the rule row shows.
#[must_use]
pub fn format_civil(at: CivilDateTime) -> String {
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}",
        at.year, at.month, at.day, at.hour, at.minute
    )
}

/// `H:MM:SS`, `MM:SS`, or plain seconds to whole milliseconds.
pub fn parse_duration(text: &str) -> Result<u64, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err(UNREADABLE.to_owned());
    }
    let parts: Vec<&str> = text.split(':').collect();
    let numbers: Vec<u64> = parts
        .iter()
        .map(|part| part.parse().map_err(|_| UNREADABLE.to_owned()))
        .collect::<Result<_, _>>()?;
    let millis = match numbers.as_slice() {
        [seconds] => seconds.saturating_mul(1000),
        [minutes, seconds] if *seconds < 60 => (minutes * 60 + seconds) * 1000,
        [hours, minutes, seconds] if *minutes < 60 && *seconds < 60 => {
            (hours * 3600 + minutes * 60 + seconds) * 1000
        }
        _ => return Err(UNREADABLE.to_owned()),
    };
    Ok(millis)
}

/// The machine's current day, as UTC (std carries no zone database; a
/// day-granular rule is insensitive to the difference for the use it serves).
#[must_use]
pub fn today() -> CivilDateTime {
    let days = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_secs() / 86_400).unwrap_or(0)
        });
    let (year, month, day) = civil_from_days(days);
    CivilDateTime {
        year,
        month,
        day,
        hour: 0,
        minute: 0,
        second: 0,
    }
}

fn days_in_month(year: i32, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap(year) => 29,
        _ => 28,
    }
}

fn is_leap(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

/// Hinnant's days-from-civil inverse, valid for the proleptic Gregorian
/// calendar.
fn civil_from_days(days: i64) -> (i32, u8, u8) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = u64::try_from(z - era * 146_097).unwrap_or(0);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = i64::try_from(yoe).unwrap_or(0) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = y + i64::from(m <= 2);
    (
        i32::try_from(year).unwrap_or(1970),
        u8::try_from(m).unwrap_or(1),
        u8::try_from(d).unwrap_or(1),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_date_round_trips_and_a_bare_day_reads_as_midnight() {
        let at = parse_civil("2024-03-05 07:08").unwrap();
        assert_eq!(format_civil(at), "2024-03-05 07:08");
        let day = parse_civil("2024-02-29").unwrap();
        assert_eq!((day.hour, day.minute), (0, 0));
        assert!(parse_civil("2023-02-29").is_err(), "not a leap day");
        assert!(parse_civil("2024-13-01").is_err());
    }

    #[test]
    fn durations_read_as_seconds_or_clock_parts() {
        assert_eq!(parse_duration("90"), Ok(90_000));
        assert_eq!(parse_duration("1:30"), Ok(90_000));
        assert_eq!(parse_duration("1:02:03"), Ok(3_723_000));
        assert!(parse_duration("1:99").is_err());
    }

    #[test]
    fn today_is_a_real_day() {
        let day = today();
        assert!(day.year >= 2024);
        assert!((1..=12).contains(&day.month));
    }
}
