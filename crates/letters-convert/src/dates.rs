//! When a file that is brought in was made and last changed, so that its document does not look new.

/// When a file was made and when it was last changed, in seconds since 1970, as far as either is known.
///
/// A reader fills in what the file says of itself: a Word file's own record, a Markdown file's front
/// matter. Whoever brings the file in knows what its place says of it, and [settles](Self::settle) the two.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Dates {
    pub created: Option<u64>,
    pub modified: Option<u64>,
}

impl Dates {
    /// These dates, and `other`'s where these say nothing: what a file says of itself comes first, and
    /// what the file system or a browser says of it fills the gaps.
    pub fn or(self, other: Self) -> Self {
        Self { created: self.created.or(other.created), modified: self.modified.or(other.modified) }
    }

    /// When the document of the file was made and last changed, at `now`. A date in the future is taken
    /// as unknown, an unknown change as made now, and the document was never made after it last changed:
    /// a copied file is often younger than what it holds.
    pub fn settle(self, now: u64) -> (u64, u64) {
        let known = |time: Option<u64>| time.filter(|time| (1..=now).contains(time));
        let modified = known(self.modified).unwrap_or(now);
        (known(self.created).unwrap_or(modified).min(modified), modified)
    }
}

/// Reads a date as files write it: `2024-03-09`, or with a time as `2024-03-09T14:05:00Z`, with a space
/// for the `T`, fractions of a second, or an offset such as `+02:00`. A time with no offset is read as UTC.
/// `None` for anything else, and for a date before 1970.
pub(crate) fn seconds(text: &str) -> Option<u64> {
    let text = text.trim().trim_matches(['"', '\'']);
    let number = |digits: &str| digits.bytes().all(|c| c.is_ascii_digit()).then(|| digits.parse::<i64>().ok())?;
    let (date, time) = match text.find(['T', 't', ' ']) {
        Some(at) => (&text[..at], Some(text[at + 1..].trim())),
        None => (text, None),
    };
    let mut parts = date.split('-');
    let (year, month, day) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() || year.len() != 4 || month.len() != 2 || day.len() != 2 {
        return None;
    }
    let (year, month, day) = (number(year)?, number(month)?, number(day)?);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    // Days since 1970, counted in years that start in March so that a leap day ends one.
    let shifted = if month <= 2 { year - 1 } else { year };
    let (era, of_era) = (shifted.div_euclid(400), shifted.rem_euclid(400));
    let of_year = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let days = era * 146_097 + of_era * 365 + of_era / 4 - of_era / 100 + of_year - 719_468;
    let mut seconds = days * 86_400;
    if let Some(time) = time {
        let zone = time.find(['Z', 'z', '+', '-']).unwrap_or(time.len());
        let (clock, zone) = time.split_at(zone);
        let mut parts = clock.split(':');
        let (hour, minute) = (number(parts.next()?)?, number(parts.next()?)?);
        let second = parts.next().map_or(Some(0), |second| number(second.split('.').next()?))?;
        if hour > 23 || minute > 59 || second > 60 || parts.next().is_some() {
            return None;
        }
        seconds += hour * 3600 + minute * 60 + second;
        if !zone.is_empty() && !zone.eq_ignore_ascii_case("z") {
            let ahead = zone.starts_with('+');
            let digits: String = zone[1..].chars().filter(|c| *c != ':').collect();
            if digits.len() != 4 {
                return None;
            }
            let offset = number(&digits[..2])? * 3600 + number(&digits[2..])? * 60;
            seconds -= if ahead { offset } else { -offset };
        }
    }
    u64::try_from(seconds).ok().filter(|seconds| *seconds > 0)
}

/// The dates a file's front matter gives, read from its lines: `date` or `created` for when it was made,
/// and `modified`, `updated` or `lastmod` for when it last changed, as the tools that write front matter
/// name them. Only a key of the outermost level counts.
pub(crate) fn of_front_matter(source: &str) -> Dates {
    let mut dates = Dates::default();
    for line in source.lines() {
        let Some((key, value)) = line.split_once([':', '=']) else { continue };
        let slot = match key.trim_end().to_ascii_lowercase().as_str() {
            "date" | "created" => &mut dates.created,
            "modified" | "updated" | "lastmod" | "last_modified" => &mut dates.modified,
            _ => continue,
        };
        *slot = slot.or_else(|| seconds(value));
    }
    dates
}
