//! When a file that is brought in was made and last changed.

use letters_convert::{Dates, import_markdown};

/// 4 May 2019 and 7 June 2021, 08:09:10, both in UTC.
const MADE: u64 = 1_556_928_000;
const CHANGED: u64 = 1_623_053_350;
const NOW: u64 = 1_800_000_000;

fn dates(created: impl Into<Option<u64>>, modified: impl Into<Option<u64>>) -> Dates {
    Dates { created: created.into(), modified: modified.into() }
}

#[test]
fn front_matter_dates_a_markdown_file() {
    let cases = [
        ("---\ndate: 2019-05-04\nmodified: 2021-06-07T08:09:10Z\n---\n", dates(MADE, CHANGED)),
        ("---\ncreated: \"2019-05-04 00:00:00\"\nupdated: 2021-06-07T10:09:10+02:00\n---\n", dates(MADE, CHANGED)),
        ("+++\ndate = 2019-05-03T19:00:00-05:00\nlastmod = 2021-06-07T08:09:10.250Z\n+++\n", dates(MADE, CHANGED)),
        ("---\ntitle: Roof\ndate: 2019-05-04\n---\n", dates(MADE, None)),
        // What is no date says nothing, and neither does a key further in.
        ("---\ndate: soon\nmodified: 2021-13-40\nbook:\n  date: 2019-05-04\n---\n", dates(None, None)),
        ("date: 2019-05-04\n", dates(None, None)),
    ];
    for (markdown, expected) in cases {
        let read = import_markdown(&format!("{markdown}\nWords.\n"), &mut String::new, &mut |_| None);
        assert_eq!(read.dates, expected, "{markdown:?}");
    }
}

#[test]
fn a_document_is_dated_by_its_file_and_then_by_where_the_file_lies() {
    // What the file says, what its place says, and when the document was made and last changed.
    let cases = [
        (dates(MADE, CHANGED), dates(NOW - 5, NOW - 5), (MADE, CHANGED)),
        (dates(None, None), dates(MADE, CHANGED), (MADE, CHANGED)),
        (dates(MADE, None), dates(None, CHANGED), (MADE, CHANGED)),
        // A copy is younger than what it holds: the document was not made after it last changed.
        (dates(None, None), dates(CHANGED, MADE), (MADE, MADE)),
        (dates(None, None), dates(None, CHANGED), (CHANGED, CHANGED)),
        // Nothing is known, or what is said has not come yet: the document is new.
        (dates(None, None), dates(None, None), (NOW, NOW)),
        (dates(NOW + 60, NOW + 60), dates(None, None), (NOW, NOW)),
        (dates(MADE, NOW + 60), dates(None, None), (MADE, NOW)),
    ];
    for (said, placed, expected) in cases {
        assert_eq!(said.or(placed).settle(NOW), expected, "{said:?} at {placed:?}");
    }
}
