use letters_trust::{HttpsUrl, Source, UrlError};

#[test]
fn an_address_is_written_back_in_one_plain_form() {
    let cases = [
        ("https://example.org", "https://example.org/"),
        ("  HTTPS://Example.ORG/Logo.png  ", "https://example.org/Logo.png"),
        ("https://example.org:443/a.png", "https://example.org/a.png"),
        ("https://example.org:8443/a.png", "https://example.org:8443/a.png"),
        ("https://example.org./a.png", "https://example.org/a.png"),
        ("https://example.org/a.png?w=200&h=100#top", "https://example.org/a.png?w=200&h=100"),
        ("https://example.org/a/./b/../c.png", "https://example.org/a/c.png"),
        ("https://example.org/a/%2E%2e/c.png", "https://example.org/c.png"),
        ("https://example.org/../../c.png", "https://example.org/c.png"),
        ("https://example.org/a/..", "https://example.org/"),
        ("https://example.org/caf\u{e9} au lait.png", "https://example.org/caf%C3%A9%20au%20lait.png"),
        ("https://example.org/a%20b.png", "https://example.org/a%20b.png"),
        ("https://my_bucket.s3.example.com/a.png", "https://my_bucket.s3.example.com/a.png"),
        ("https://93.184.216.34/a.png", "https://93.184.216.34/a.png"),
        ("https://[2606:4700:4700::1111]:8443/a.png", "https://[2606:4700:4700::1111]:8443/a.png"),
    ];
    for (text, written) in cases {
        let url = HttpsUrl::parse(text).unwrap_or_else(|error| panic!("{text}: {error}"));
        assert_eq!(url.to_string(), written, "{text}");
        assert_eq!(HttpsUrl::parse(written), Ok(url), "{written} reads back as itself");
    }
}

#[test]
fn an_address_that_could_be_misread_or_misused_is_refused() {
    let cases = [
        ("http://example.org/a.png", UrlError::NotHttps),
        ("ftp://example.org/a.png", UrlError::NotHttps),
        ("//example.org/a.png", UrlError::NotHttps),
        ("example.org/a.png", UrlError::NotHttps),
        ("data:image/png;base64,AAAA", UrlError::NotHttps),
        ("file:///etc/passwd", UrlError::NotHttps),
        ("", UrlError::NotHttps),
        // A user name disguises the host.
        ("https://example.org@evil.example/a.png", UrlError::Credentials),
        ("https://user:pass@example.org/a.png", UrlError::Credentials),
        // Spellings of an address that only some programs read as one.
        ("https://2130706433/a.png", UrlError::Malformed),
        ("https://127.1/a.png", UrlError::Malformed),
        ("https://0x7f.0.0.1/a.png", UrlError::Malformed),
        ("https://0177.0.0.1/a.png", UrlError::Malformed),
        ("https://127.0.0.1.0x10/a.png", UrlError::Malformed),
        ("https://%31%32%37.0.0.1/a.png", UrlError::Malformed),
        ("https://[::1%25eth0]/a.png", UrlError::Malformed),
        // Hosts that programs split differently.
        ("https://example.org\\@evil.example/a.png", UrlError::Malformed),
        ("https:\\\\example.org/a.png", UrlError::Malformed),
        ("https:/example.org/a.png", UrlError::Malformed),
        ("https:example.org/a.png", UrlError::Malformed),
        ("https://exa\tmple.org/a.png", UrlError::Malformed),
        ("https://example.org/a\n.png", UrlError::Malformed),
        ("https://exam ple.org/a.png", UrlError::Malformed),
        ("https://ex\u{e4}mple.org/a.png", UrlError::Malformed),
        ("https://example..org/a.png", UrlError::Malformed),
        ("https:///a.png", UrlError::Malformed),
        ("https://example.org:/a.png", UrlError::Malformed),
        ("https://example.org:0/a.png", UrlError::Malformed),
        ("https://example.org:+443/a.png", UrlError::Malformed),
        ("https://example.org:99999/a.png", UrlError::Malformed),
        ("https://example.org:44:3/a.png", UrlError::Malformed),
        // A broken escape.
        ("https://example.org/100%.png", UrlError::Malformed),
        ("https://example.org/a.png?q=%zz", UrlError::Malformed),
    ];
    for (text, error) in cases {
        assert_eq!(HttpsUrl::parse(text), Err(error), "{text:?}");
    }
}

#[test]
fn an_address_on_this_device_or_a_private_network_is_told_apart() {
    let cases = [
        ("https://example.org/a.png", true),
        ("https://93.184.216.34/a.png", true),
        ("https://[2606:4700:4700::1111]/a.png", true),
        // A name says nothing about where it resolves: the fetch checks that when it connects.
        ("https://nas.tailnet.example/a.png", true),
        ("https://localhost/a.png", false),
        ("https://LOCALHOST./a.png", false),
        ("https://app.localhost:8443/a.png", false),
        ("https://127.0.0.1/a.png", false),
        ("https://10.1.2.3/a.png", false),
        ("https://192.168.0.1/a.png", false),
        ("https://169.254.169.254/latest/meta-data/", false),
        ("https://100.100.100.100/a.png", false),
        ("https://[::1]/a.png", false),
        ("https://[::ffff:127.0.0.1]/a.png", false),
        ("https://[fd00::1]/a.png", false),
        ("https://0.0.0.0/a.png", false),
    ];
    for (text, public) in cases {
        let url = HttpsUrl::parse(text).unwrap_or_else(|error| panic!("{text}: {error}"));
        assert_eq!(url.is_public(), public, "{text}");
    }
}

#[test]
fn a_source_allows_its_host_or_prefix_and_nothing_near_it() {
    let cases = [
        // A whole host.
        ("https://upload.wikimedia.org/", "https://upload.wikimedia.org/a/b.png", true),
        ("upload.wikimedia.org", "https://UPLOAD.wikimedia.org/a/b.png?w=1", true),
        ("https://upload.wikimedia.org/", "https://wikimedia.org/a.png", false),
        ("https://wikimedia.org/", "https://upload.wikimedia.org/a.png", false),
        ("https://wikimedia.org/", "https://evilwikimedia.org/a.png", false),
        ("https://wikimedia.org/", "https://wikimedia.org.evil.example/a.png", false),
        ("https://wikimedia.org/", "https://wikimedia.org:8443/a.png", false),
        ("https://wikimedia.org:8443/", "https://wikimedia.org:8443/a.png", true),
        // A prefix.
        ("https://img.example.org/charts/", "https://img.example.org/charts/q3.png", true),
        ("https://img.example.org/charts/", "https://img.example.org/charts/2026/q3.png", true),
        ("https://img.example.org/charts", "https://img.example.org/charts/q3.png", true),
        ("https://img.example.org/charts", "https://img.example.org/charts", true),
        ("https://img.example.org/charts", "https://img.example.org/charts-private/q3.png", false),
        ("https://img.example.org/charts/", "https://img.example.org/private/q3.png", false),
        ("https://img.example.org/charts/", "https://img.example.org/Charts/q3.png", false),
        // Climbing out of the prefix.
        ("https://img.example.org/charts/", "https://img.example.org/charts/../private/q3.png", false),
        ("https://img.example.org/charts/", "https://img.example.org/charts/%2e%2e/private/q3.png", false),
        ("https://img.example.org/charts/", "https://img.example.org/charts/..%2Fprivate/q3.png", false),
        ("https://img.example.org/charts/", "https://img.example.org/charts/..%5cprivate/q3.png", false),
        ("https://img.example.org/charts/", "https://img.example.org/charts/a/../q3.png", true),
        // A source on a private network.
        ("https://192.168.1.10/photos/", "https://192.168.1.10/photos/a.png", true),
        ("https://192.168.1.10/photos/", "https://192.168.1.11/photos/a.png", false),
        ("nas.tailnet.example", "https://nas.tailnet.example/a.png", true),
    ];
    for (source, url, allowed) in cases {
        let parsed = Source::parse(source).unwrap_or_else(|error| panic!("{source}: {error}"));
        let address = HttpsUrl::parse(url).unwrap_or_else(|error| panic!("{url}: {error}"));
        assert_eq!(parsed.allows(&address), allowed, "{source} and {url}");
    }
}

#[test]
fn a_source_must_be_an_https_host_or_prefix() {
    let cases = [
        ("http://img.example.org/", UrlError::NotHttps),
        ("https://user@img.example.org/", UrlError::Credentials),
        ("https://img.example.org/charts?size=large", UrlError::Malformed),
        ("https://img.example.org/charts#top", UrlError::Malformed),
        ("*.example.org", UrlError::Malformed),
        ("", UrlError::Malformed),
    ];
    for (text, error) in cases {
        assert_eq!(Source::parse(text), Err(error), "{text:?}");
    }
}

#[test]
fn the_host_of_an_address_is_a_source_for_everything_on_it() {
    let url = HttpsUrl::parse("https://img.example.org:8443/charts/q3.png?w=200").unwrap();
    let host = Source::host_of(&url);
    assert_eq!(host.to_string(), "https://img.example.org:8443/");
    assert!(host.allows(&HttpsUrl::parse("https://img.example.org:8443/other.png").unwrap()));
    assert_eq!(Source::parse(&host.to_string()), Ok(host), "a source reads back from its written form");
}
