//! The card `i` raises: the change behind a line, drawn by the hover card's
//! renderer.

use rv_core::vcs::LineOrigin;

use super::short_change_id;
use crate::app::HoverCard;
use crate::app::JumpSource;

const SECONDS_PER_DAY: i64 = 86_400;

pub(super) fn origin_card(origin: &LineOrigin, line: u32, as_of_secs: i64) -> HoverCard {
    let author = match printable(&origin.author) {
        name if name.is_empty() => "(no author)".to_owned(),
        name => name,
    };
    let description = match printable(&origin.description_first_line) {
        text if text.is_empty() => "(no description set)".to_owned(),
        text => text,
    };
    let commit: String = origin.commit_id.chars().take(12).collect();
    HoverCard {
        name: format!("change {}", short_change_id(&origin.change_id)),
        kind: None,
        declaration: None,
        references: 0,
        source: JumpSource::Annotation,
        server_text: vec![
            description,
            format!(
                "{author} · {} ({})",
                utc_date(origin.timestamp_secs),
                age(origin.timestamp_secs, as_of_secs)
            ),
            format!("commit {commit}"),
        ],
        note: None,
        hints: format!("line {line} · Esc close"),
    }
}

fn printable(text: &str) -> String {
    text.chars()
        .filter(|character| !character.is_control())
        .collect::<String>()
        .trim()
        .to_owned()
}

/// `YYYY-MM-DD HH:MM UTC`, by the proleptic Gregorian calendar.
pub(super) fn utc_date(secs: i64) -> String {
    let days = secs.div_euclid(SECONDS_PER_DAY);
    let minutes = secs.rem_euclid(SECONDS_PER_DAY) / 60;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02} UTC",
        minutes / 60,
        minutes % 60
    )
}

/// Howard Hinnant's `civil_from_days`: days since 1970-01-01 to a date.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

pub(super) fn age(secs: i64, as_of_secs: i64) -> String {
    let elapsed = as_of_secs - secs;
    if elapsed < 60 {
        return "just now".to_owned();
    }
    let (count, unit) = match elapsed {
        60..3_600 => (elapsed / 60, "minute"),
        3_600..SECONDS_PER_DAY => (elapsed / 3_600, "hour"),
        SECONDS_PER_DAY..2_592_000 => (elapsed / SECONDS_PER_DAY, "day"),
        2_592_000..31_536_000 => (elapsed / 2_592_000, "month"),
        _ => (elapsed / 31_536_000, "year"),
    };
    let plural = if count == 1 { "" } else { "s" };
    format!("{count} {unit}{plural} ago")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_are_utc_calendar_dates() {
        assert_eq!(utc_date(0), "1970-01-01 00:00 UTC");
        assert_eq!(utc_date(951_782_400), "2000-02-29 00:00 UTC");
        assert_eq!(utc_date(1_700_000_000), "2023-11-14 22:13 UTC");
        assert_eq!(utc_date(-86_400), "1969-12-31 00:00 UTC");
    }

    #[test]
    fn ages_name_the_largest_whole_unit() {
        assert_eq!(age(100, 130), "just now");
        assert_eq!(age(0, 60), "1 minute ago");
        assert_eq!(age(0, 7_200), "2 hours ago");
        assert_eq!(age(0, 3 * SECONDS_PER_DAY), "3 days ago");
        assert_eq!(age(0, 400 * SECONDS_PER_DAY), "1 year ago");
    }
}
