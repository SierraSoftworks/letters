mod common;

use std::net::IpAddr;

use common::{PRIVATE_SOURCES, SETTINGS, safe, trust, trusted};
use letters_trust::{FetchError, ImageFetch, ImageType, Limits, UrlError};

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR";
const SMALL: Limits = Limits { max_bytes: 1024, max_redirects: 2 };

/// A fetch the user asked for, in a trusted document.
fn asked(url: &str) -> ImageFetch {
    trusted(&trust(SETTINGS)).fetch_on_request(url, SMALL).expect("the fetch may start")
}

/// A fetch on open from a trusted source, in a trusted document.
fn on_open(url: &str) -> ImageFetch {
    trusted(&trust(SETTINGS)).fetch_on_open(url, SMALL).expect("the fetch may start")
}

#[test]
fn an_image_from_a_public_host_is_fetched() {
    let mut fetch = on_open("https://UPLOAD.wikimedia.org/a/../logo.png#top");
    assert_eq!(fetch.url().to_string(), "https://upload.wikimedia.org/logo.png", "the plain form is requested");
    fetch.connect("208.80.154.240".parse().unwrap()).unwrap();
    fetch.response(Some("image/png"), Some(PNG.len() as u64)).unwrap();
    fetch.receive(PNG.len()).unwrap();
    let kind = fetch.finish(PNG).unwrap();
    assert_eq!(kind, ImageType::Png);
    assert_eq!(kind.media_type(), "image/png");
}

#[test]
fn a_fetch_starts_only_when_the_document_and_the_source_allow_it() {
    let trust = trust(SETTINGS);
    let source = "https://upload.wikimedia.org/a.png";
    let elsewhere = "https://tracker.example/pixel.gif";
    let private = "https://192.168.1.1/a.png";
    let cases = [
        (trusted(&trust).fetch_on_open(source, SMALL), Ok(())),
        (trusted(&trust).fetch_on_request(source, SMALL), Ok(())),
        (trusted(&trust).fetch_on_request(elsewhere, SMALL), Ok(())),
        (trusted(&trust).fetch_on_open(elsewhere, SMALL), Err(FetchError::UntrustedSource)),
        (trusted(&trust).fetch_on_request(private, SMALL), Err(FetchError::Url(UrlError::Private))),
        (
            trusted(&trust).fetch_on_open("http://upload.wikimedia.org/a.png", SMALL),
            Err(FetchError::Url(UrlError::NotHttps)),
        ),
        // Safe mode reaches the network for nothing, asked or not.
        (safe(&trust).fetch_on_open(source, SMALL), Err(FetchError::Held)),
        (safe(&trust).fetch_on_request(source, SMALL), Err(FetchError::Held)),
        (safe(&trust).fetch_on_request(elsewhere, SMALL), Err(FetchError::Held)),
    ];
    for (at, (fetch, expected)) in cases.into_iter().enumerate() {
        assert_eq!(fetch.map(drop), expected, "case {at}");
    }
}

#[test]
fn a_host_that_resolves_to_a_private_address_is_not_connected_to() {
    let fetch = asked("https://rebind.example/a.png");
    for address in ["127.0.0.1", "10.0.0.5", "169.254.169.254", "100.64.0.7", "::1", "fd00::1", "::ffff:192.168.1.1"] {
        let address: IpAddr = address.parse().unwrap();
        assert_eq!(fetch.connect(address), Err(FetchError::PrivateAddress(address)));
    }
    assert_eq!(fetch.connect("93.184.216.34".parse().unwrap()), Ok(()));
}

#[test]
fn a_trusted_source_may_be_on_a_private_address() {
    let trust = trust(PRIVATE_SOURCES);
    let tailnet: IpAddr = "100.64.0.7".parse().unwrap();

    // A name that resolves into the tailnet, and an address written out.
    let named = trusted(&trust).fetch_on_open("https://nas.tailnet.example/a.png", SMALL).unwrap();
    assert_eq!(named.connect(tailnet), Ok(()));
    let written = trusted(&trust).fetch_on_open("https://192.168.1.10/photos/a.png", SMALL).unwrap();
    assert_eq!(written.connect("192.168.1.10".parse().unwrap()), Ok(()));

    // Another host gains nothing from it, unless it redirects into the source itself.
    let mut other = trusted(&trust).fetch_on_request("https://example.org/a.png", SMALL).unwrap();
    assert_eq!(other.connect(tailnet), Err(FetchError::PrivateAddress(tailnet)));
    assert_eq!(other.redirect("https://192.168.1.10/admin/a.png"), Err(FetchError::Url(UrlError::Private)));
    assert_eq!(other.redirect("https://192.168.1.10/photos/a.png"), Ok(()));
    assert_eq!(other.connect("192.168.1.10".parse().unwrap()), Ok(()));

    // Safe mode holds it like any other image, and a server does not take a user's word for it.
    let held = safe(&trust).fetch_on_request("https://nas.tailnet.example/a.png", SMALL);
    assert_eq!(held.map(drop), Err(FetchError::Held));
    let on_server = trusted(&trust).on_server(&[]).fetch_on_open("https://nas.tailnet.example/a.png", SMALL).unwrap();
    assert_eq!(on_server.connect(tailnet), Err(FetchError::PrivateAddress(tailnet)));
}

#[test]
fn a_redirect_is_held_to_the_rules_of_the_first_address() {
    let from = "https://upload.wikimedia.org/thumb/a.png";
    let cases = [
        ("https://cdn.example.net/a.png", Ok("https://cdn.example.net/a.png")),
        ("/full/a.png", Ok("https://upload.wikimedia.org/full/a.png")),
        ("b.png?w=2", Ok("https://upload.wikimedia.org/thumb/b.png?w=2")),
        ("../b.png", Ok("https://upload.wikimedia.org/b.png")),
        ("?w=2", Ok("https://upload.wikimedia.org/thumb/a.png?w=2")),
        ("//cdn.example.net/a.png", Ok("https://cdn.example.net/a.png")),
        ("http://upload.wikimedia.org/a.png", Err(FetchError::Url(UrlError::NotHttps))),
        ("https://127.0.0.1/admin", Err(FetchError::Url(UrlError::Private))),
        ("https://localhost:8443/", Err(FetchError::Url(UrlError::Private))),
        ("//169.254.169.254/latest/meta-data/", Err(FetchError::Url(UrlError::Private))),
        ("https://user@cdn.example.net/a.png", Err(FetchError::Url(UrlError::Credentials))),
        ("file:///etc/passwd", Err(FetchError::Url(UrlError::NotHttps))),
    ];
    for (location, expected) in cases {
        let mut fetch = asked(from);
        let moved = fetch.redirect(location).map(|()| fetch.url().to_string());
        assert_eq!(moved, expected.map(String::from), "{location}");
        if expected.is_err() {
            assert_eq!(fetch.url().to_string(), from, "a refused redirect moves nothing");
        }
    }
}

#[test]
fn a_fetch_on_open_may_not_be_redirected_out_of_the_trusted_sources() {
    let cases = [
        ("https://upload.wikimedia.org/full/a.png", Ok(())),
        ("https://img.example.org/charts/a.png", Ok(())),
        ("https://img.example.org/private/a.png", Err(FetchError::UntrustedSource)),
        ("https://tracker.example/pixel.gif?reader=ada", Err(FetchError::UntrustedSource)),
    ];
    for (location, expected) in cases {
        assert_eq!(on_open("https://upload.wikimedia.org/a.png").redirect(location), expected, "{location}");
    }
}

#[test]
fn redirects_are_counted() {
    let mut fetch = asked("https://example.org/0");
    for hop in 1..=SMALL.max_redirects {
        fetch.redirect(&format!("/{hop}")).unwrap();
    }
    assert_eq!(fetch.redirect("/one-too-many"), Err(FetchError::TooManyRedirects));
}

#[test]
fn a_body_over_the_limit_is_cut_off_wherever_it_shows() {
    let limit = SMALL.max_bytes;
    let too_large = Err(FetchError::TooLarge(limit));

    let fetch = asked("https://example.org/a.png");
    assert_eq!(fetch.response(Some("image/png"), Some(limit)), Ok(()));
    assert_eq!(fetch.response(Some("image/png"), Some(limit + 1)), too_large);

    // A server that says nothing, or lies, about the length.
    let mut streamed = asked("https://example.org/a.png");
    streamed.response(Some("image/png"), None).unwrap();
    assert_eq!(streamed.receive(1000), Ok(()));
    assert_eq!(streamed.receive(24), Ok(()));
    assert_eq!(streamed.receive(1), too_large);

    let mut body = PNG.to_vec();
    body.resize(limit as usize + 1, 0);
    assert_eq!(asked("https://example.org/a.png").finish(&body).map(drop), too_large.map(drop));
}

#[test]
fn only_a_response_that_claims_to_be_an_image_is_read() {
    let cases = [
        (Some("image/png"), true),
        (Some("IMAGE/JPEG"), true),
        (Some("image/gif; charset=binary"), true),
        (Some("image/webp"), true),
        (Some("application/octet-stream"), true),
        (None, true),
        (Some("text/html; charset=utf-8"), false),
        (Some("image/svg+xml"), false),
        (Some("application/json"), false),
        (Some("image/tiff"), false),
    ];
    for (content_type, read) in cases {
        let result = asked("https://example.org/a").response(content_type, None);
        assert_eq!(result.is_ok(), read, "{content_type:?}");
    }
}

#[test]
fn the_bytes_decide_what_the_image_is() {
    let cases: [(&[u8], Option<ImageType>); 9] = [
        (PNG, Some(ImageType::Png)),
        (b"\xff\xd8\xff\xe0\0\x10JFIF", Some(ImageType::Jpeg)),
        (b"GIF89a\x01\0\x01\0", Some(ImageType::Gif)),
        (b"GIF87a\x01\0\x01\0", Some(ImageType::Gif)),
        (b"RIFF\x24\0\0\0WEBPVP8 ", Some(ImageType::Webp)),
        (b"<svg xmlns='http://www.w3.org/2000/svg'><script>alert(1)</script></svg>", None),
        (b"<!doctype html><title>Sign in</title>", None),
        (b"RIFF\x24\0\0\0WAVEfmt ", None),
        (b"", None),
    ];
    for (body, kind) in cases {
        // The label says PNG every time: it is not evidence.
        let fetch = asked("https://example.org/a.png");
        fetch.response(Some("image/png"), None).unwrap();
        assert_eq!(fetch.finish(body).ok(), kind, "{:?}", String::from_utf8_lossy(body));
    }
}
