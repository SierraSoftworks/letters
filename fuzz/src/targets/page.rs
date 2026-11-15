//! A page fetched for a link preview: whatever it holds, what is read of it fits a preview.

use letters_convert::page_summary;
use letters_model::BlockKind;

pub fn run(data: &[u8]) {
    let read = page_summary(data);
    let lengths = [(&read.title, 200), (&read.description, 300), (&read.image, 2048)];
    for (text, longest) in lengths {
        let length = text.as_ref().map_or(0, |text| text.chars().count());
        assert!(length <= longest, "a page made a preview hold {length} characters");
    }
    assert!(read.image.as_ref().is_none_or(|address| !address.contains(char::is_control)), "an address has a line end");
    // The picture is checked and kept as an asset by the shell, so the rest is what the document takes.
    let kind = BlockKind::LinkPreview {
        url: "https://example.com/".into(),
        title: read.title,
        description: read.description,
        image: None,
    };
    assert!(kind.sound(), "a page made a preview no document can hold");
}

pub fn seeds() -> Vec<Vec<u8>> {
    let carded = concat!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><title>Fees &amp; dues | Harbour</title>",
        "<meta name=\"description\" content=\"Plain\"><meta property=\"og:title\" content=\"Harbour fees\">",
        "<meta property=\"og:description\" content=\"What a night on the quay costs,\n by length.\">",
        "<meta property=\"og:image\" content=\"/quay.jpg\"><meta name=\"twitter:image\" content=\"https://example.com/q.png\">",
        "<script>const t = '<title>no</title>';</script></head><body><h1>Fees</h1></body></html>",
    );
    let bare = "<title>Tides</title><p>High water is at noon.";
    vec![carded.into(), bare.into(), b"<head><title>Caf\xe9</title><meta name=description content='\xff'>".to_vec()]
}
