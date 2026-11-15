//! What a fetched page says of itself, as a link preview takes it.

use letters_convert::{PageSummary, page_summary};
use proptest::prelude::*;

/// A page whose head holds `head`.
fn page(head: &str) -> String {
    format!("<!doctype html><html><head>{head}</head><body><h1>Harbour</h1></body></html>")
}

fn summary(title: Option<&str>, description: Option<&str>, image: Option<&str>) -> PageSummary {
    let text = |text: Option<&str>| text.map(String::from);
    PageSummary { title: text(title), description: text(description), image: text(image) }
}

#[test]
fn reads_what_a_page_says_of_itself() {
    let cases = [
        // What a page says for a card comes before what it says for all, whatever the order.
        (
            r#"<title>Fees | Harbour</title><meta name="description" content="Plain">
               <meta name="twitter:title" content="Tweeted"><meta property="og:title" content="Harbour fees">
               <meta property="og:description" content="What a night costs.">
               <meta property="og:image" content="/quay.jpg">"#,
            summary(Some("Harbour fees"), Some("What a night costs."), Some("/quay.jpg")),
        ),
        (
            r#"<title>Fees</title><meta name="twitter:description" content="Tweeted">
               <meta name="twitter:image" content="https://example.com/quay.png">"#,
            summary(Some("Fees"), Some("Tweeted"), Some("https://example.com/quay.png")),
        ),
        (r#"<title>Fees</title><meta name="description" content="Plain">"#, summary(Some("Fees"), Some("Plain"), None)),
        // Names are read whatever their case, and under either attribute.
        (r#"<META NAME="OG:Title" CONTENT="Harbour fees">"#, summary(Some("Harbour fees"), None, None)),
        // Text is plain text on one line, and its escapes are read.
        (
            "<title>\n  Fees &amp; <b>dues</b>\n</title><meta name=description content='A &quot;night&quot;\n on\tthe quay'>",
            summary(Some("Fees & <b>dues</b>"), Some("A \"night\" on the quay"), None),
        ),
        // The first of a name stands, and one that says nothing does not count.
        (
            r#"<meta property="og:title" content=" "><title>Fees</title><title>Other</title>
               <meta property="og:image" content="a.png"><meta property="og:image" content="b.png">"#,
            summary(Some("Fees"), None, Some("a.png")),
        ),
        ("", summary(None, None, None)),
    ];
    for (head, expected) in cases {
        assert_eq!(page_summary(page(head).as_bytes()), expected, "reading {head}");
    }
}

#[test]
fn reads_only_what_the_head_says() {
    let said = r#"<meta property="og:title" content="Planted"><title>Planted</title>"#;
    let cases = [
        format!("<html><head><title>Fees</title></head><body>{said}</body></html>"),
        // A page may leave its head's tags out.
        format!("<title>Fees</title><body>{said}"),
        // What a script or a style sheet holds is not the page's own word.
        page(&format!("<script>const card = '{said}';</script><style>/* {said} */</style><title>Fees</title>")),
    ];
    for html in cases {
        assert_eq!(page_summary(html.as_bytes()).title.as_deref(), Some("Fees"), "reading {html}");
    }
}

#[test]
fn keeps_no_more_of_a_page_than_a_card_shows() {
    let long = "harbour ".repeat(10_000);
    let read = page_summary(page(&format!("<title>{long}</title><meta name=description content='{long}'>")).as_bytes());
    for text in [read.title.expect("a title"), read.description.expect("a description")] {
        assert!(text.chars().count() <= 300 && text.ends_with("harbour\u{2026}"), "cut at a word: {text}");
    }
    // A word with no end is cut all the same, and an address that long is none.
    let endless = "a".repeat(100_000);
    let head = format!("<title>{endless}</title><meta property='og:image' content='https://example.com/{endless}'>");
    let read = page_summary(page(&head).as_bytes());
    assert!(read.title.is_some_and(|title| title.chars().count() <= 300));
    assert_eq!(read.image, None);
}

#[test]
fn leaves_out_text_it_cannot_read() {
    // A page in another encoding than UTF-8: its title cannot be told, and the rest still can.
    let mut html = b"<head><title>Caf\xe9</title><meta property='og:image' content='quay.png'>".to_vec();
    html.extend_from_slice(b"<meta name='description' content='Plain'></head>");
    assert_eq!(page_summary(&html), summary(None, Some("Plain"), Some("quay.png")));
}

proptest! {
    /// Whatever a page holds, what is read of it is fit for a link preview: plain text on one line.
    #[test]
    fn whatever_a_page_holds_is_read_as_lines(parts in proptest::collection::vec(prop_oneof![
        "[a-z<>/=\"' &;\n]{0,12}",
        proptest::sample::select(vec![
            "<title>", "</title>", "<meta property=\"og:title\" content=\"", "<meta name=description content='",
            "<meta property=og:image content=", "<head>", "</head>", "<body>", "<script>", "</script>", "\">", "'>",
            "&amp;", "&#0;", "\u{0}", "\r\n", "\u{2028}", "é",
        ]).prop_map(str::to_owned),
    ], 0..24)) {
        let read = page_summary(parts.concat().as_bytes());
        for text in [&read.title, &read.description, &read.image].into_iter().flatten() {
            prop_assert!(!text.is_empty() && !text.contains(char::is_control), "{text:?}");
            prop_assert_eq!(text.trim(), text.as_str());
        }
    }
}
