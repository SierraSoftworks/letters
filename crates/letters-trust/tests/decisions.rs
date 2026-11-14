mod common;

use common::{HOME, PRIVATE_SOURCES, SETTINGS, safe, trust, trusted};
use letters_trust::{ImageDecision, LinkDecision, MergeDecision, Mode, Origin, Source, Trust, UrlError};

#[test]
fn a_document_is_trusted_by_who_made_it_its_id_or_its_folder() {
    let trust = trust(SETTINGS);
    let cases = [
        ("doc-new", Origin::Created, Mode::Trusted),
        ("doc-trusted", Origin::Outside, Mode::Trusted),
        ("doc-trusted", Origin::File("/home/ada/Downloads/report.let"), Mode::Trusted),
        ("doc-unknown", Origin::File("/home/ada/Documents/report.let"), Mode::Trusted),
        ("doc-unknown", Origin::File("/home/ada/Documents/work/2026/report.let"), Mode::Trusted),
        ("doc-unknown", Origin::File("/srv/shared/report.let"), Mode::Trusted),
        ("doc-unknown", Origin::File("/home/ada/Downloads/report.let"), Mode::Safe),
        // A download or a share link is not vouched for by where it was saved.
        ("doc-unknown", Origin::Outside, Mode::Safe),
        ("", Origin::Outside, Mode::Safe),
    ];
    for (id, origin, mode) in cases {
        assert_eq!(trust.mode(id, origin), mode, "{id} from {origin:?}");
    }
}

#[test]
fn nothing_is_trusted_without_settings() {
    let trust = Trust::default();
    assert_eq!(trust.mode("doc-trusted", Origin::File("/home/ada/Documents/report.let")), Mode::Safe);
    assert_eq!(trust.mode("doc-trusted", Origin::Outside), Mode::Safe);
}

#[test]
fn a_trusted_folder_covers_what_is_inside_it_and_nothing_beside_it() {
    let cases = [
        ("~/Documents", "/home/ada/Documents/a.let", true),
        ("~/Documents/", "/home/ada/Documents/deep/er/a.let", true),
        ("~", "/home/ada/anything/a.let", true),
        ("/srv/shared", "/srv/shared/a.let", true),
        ("/srv//shared/./", "/srv/shared/a.let", true),
        ("~/Documents", "/home/ada/Documents-old/a.let", false),
        ("~/Documents", "/home/ada/documents/a.let", false),
        ("~/Documents", "/home/eve/Documents/a.let", false),
        ("~/Documents", "/tmp/home/ada/Documents/a.let", false),
        // A path that climbs is never trusted: the shell resolves it first.
        ("~/Documents", "/home/ada/Documents/../Downloads/a.let", false),
        ("~/Documents", "Documents/a.let", false),
        // Entries that would trust everything, or nothing that can be found, are ignored.
        ("/", "/home/ada/Documents/a.let", false),
        ("Documents", "/home/ada/Documents/a.let", false),
        ("~ada/Documents", "/home/ada/Documents/a.let", false),
        ("/home/ada/Documents/../..", "/home/ada/a.let", false),
        // Windows paths take either separator and ignore case.
        (r"C:\Users\Ada\Documents", r"C:\Users\Ada\Documents\a.let", true),
        (r"C:\Users\Ada\Documents", r"\\?\c:\users\ada\documents\a.let", true),
        ("C:/Users/Ada/Documents", r"C:\Users\Ada\Documents\a.let", true),
        (r"C:\Users\Ada\Documents", r"D:\Users\Ada\Documents\a.let", false),
        (r"C:\", r"C:\Users\Ada\a.let", false),
        (r"\\nas\share\docs", r"\\nas\share\docs\a.let", true),
        (r"\\nas\share\docs", r"\\other\share\docs\a.let", false),
    ];
    for (folder, path, trusted) in cases {
        let settings = format!("[trust]\nfolders = ['{folder}']");
        assert_eq!(trust(&settings).trusts_folder(path), trusted, "{path} in {folder}");
    }
}

#[test]
fn a_home_folder_is_needed_to_read_a_tilde() {
    let trust = Trust::parse("[trust]\nfolders = ['~/Documents', '/srv/shared']", None).unwrap();
    assert!(!trust.trusts_folder("/home/ada/Documents/a.let"));
    assert!(trust.trusts_folder("/srv/shared/a.let"));
    assert_eq!(trust.folders(), ["/srv/shared"]);
    assert_eq!(trust.ignored().len(), 1);
}

#[test]
fn an_image_loads_from_a_trusted_source_in_a_trusted_document() {
    let trust = trust(SETTINGS);
    let cases = [
        ("https://upload.wikimedia.org/a.png", ImageDecision::Load, ImageDecision::Held),
        ("https://img.example.org/charts/q3.png", ImageDecision::Load, ImageDecision::Held),
        ("https://img.example.org/private/q3.png", ImageDecision::OnRequest, ImageDecision::Held),
        ("https://tracker.example/pixel.gif?reader=ada", ImageDecision::OnRequest, ImageDecision::Held),
        // Refused in any document.
        (
            "http://upload.wikimedia.org/a.png",
            ImageDecision::Refused(UrlError::NotHttps),
            ImageDecision::Refused(UrlError::NotHttps),
        ),
        (
            "https://192.168.1.1/a.png",
            ImageDecision::Refused(UrlError::Private),
            ImageDecision::Refused(UrlError::Private),
        ),
        (
            "https://upload.wikimedia.org@evil.example/a.png",
            ImageDecision::Refused(UrlError::Credentials),
            ImageDecision::Refused(UrlError::Credentials),
        ),
    ];
    for (url, in_trusted, in_safe) in cases {
        assert_eq!(trusted(&trust).image(url), in_trusted, "{url} in a trusted document");
        assert_eq!(safe(&trust).image(url), in_safe, "{url} in safe mode");
    }
}

#[test]
fn a_private_address_loads_only_from_a_trusted_source() {
    use ImageDecision::{Held, Load, Refused};
    let trust = trust(PRIVATE_SOURCES);
    let cases = [
        ("https://192.168.1.10/photos/a.png", Load, Held),
        ("https://localhost:8443/a.png", Load, Held),
        // Beside the source, and elsewhere on the network.
        ("https://192.168.1.10/admin/a.png", Refused(UrlError::Private), Refused(UrlError::Private)),
        ("https://192.168.1.10:8443/photos/a.png", Refused(UrlError::Private), Refused(UrlError::Private)),
        ("https://192.168.1.11/photos/a.png", Refused(UrlError::Private), Refused(UrlError::Private)),
        ("https://localhost/a.png", Refused(UrlError::Private), Refused(UrlError::Private)),
        ("http://192.168.1.10/photos/a.png", Refused(UrlError::NotHttps), Refused(UrlError::NotHttps)),
    ];
    for (url, in_trusted, in_safe) in cases {
        assert_eq!(trusted(&trust).image(url), in_trusted, "{url} in a trusted document");
        assert_eq!(safe(&trust).image(url), in_safe, "{url} in safe mode");
    }
}

#[test]
fn the_servers_sources_load_for_every_user() {
    let trust = trust(SETTINGS);
    let server = [Source::parse("https://cdn.example.net/").unwrap()];
    let url = "https://cdn.example.net/a.png";
    assert_eq!(trusted(&trust).image(url), ImageDecision::OnRequest);
    assert_eq!(trusted(&trust).on_server(&server).image(url), ImageDecision::Load);
    assert_eq!(trusted(&trust).on_server(&server).image("https://upload.wikimedia.org/a.png"), ImageDecision::Load);
    assert_eq!(safe(&trust).on_server(&server).image(url), ImageDecision::Held, "safe mode still holds it");
}

#[test]
fn on_a_server_only_its_administrator_vouches_for_a_private_address() {
    let trust = trust(PRIVATE_SOURCES);
    let server =
        [Source::parse("https://assets.internal.example/").unwrap(), Source::parse("https://10.0.0.5/").unwrap()];
    let gate = trusted(&trust).on_server(&server);
    // The user's own source would point the server at its neighbours.
    assert_eq!(gate.image("https://192.168.1.10/photos/a.png"), ImageDecision::Refused(UrlError::Private));
    assert_eq!(gate.image("https://10.0.0.5/a.png"), ImageDecision::Load);
}

#[test]
fn a_link_opens_asks_or_is_refused() {
    use LinkDecision::{Ask, Open, Refuse};
    let trust = trust(SETTINGS);
    let cases = [
        ("https://example.org/page", Open, Ask),
        ("http://example.org/page", Open, Ask),
        ("HTTPS://EXAMPLE.ORG", Open, Ask),
        ("mailto:ada@example.org", Open, Ask),
        // A place in this document never leaves it.
        ("#conclusions", Open, Open),
        // The host is not the one that shows first.
        ("https://bank.example@evil.example/login", Ask, Ask),
        ("http://bank.example:secret@evil.example/", Ask, Ask),
        // Another app's scheme, or no scheme at all.
        ("zotero://select/items/1", Ask, Ask),
        ("tel:+15551234", Ask, Ask),
        ("example.org/page", Ask, Ask),
        ("../other.let", Ask, Ask),
        // These run something here.
        ("javascript:alert(1)", Refuse, Refuse),
        ("JaVaScRiPt:alert(1)", Refuse, Refuse),
        ("java\tscript:alert(1)", Refuse, Refuse),
        ("vbscript:msgbox(1)", Refuse, Refuse),
        ("data:text/html,<script>alert(1)</script>", Refuse, Refuse),
        ("file:///Applications/Calculator.app", Refuse, Refuse),
        ("", Refuse, Refuse),
    ];
    for (href, in_trusted, in_safe) in cases {
        assert_eq!(trusted(&trust).link(href), in_trusted, "{href:?} in a trusted document");
        assert_eq!(safe(&trust).link(href), in_safe, "{href:?} in safe mode");
    }
}

#[test]
fn maths_may_link_only_in_a_trusted_document() {
    let trust = trust(SETTINGS);
    assert!(trusted(&trust).maths_links());
    assert!(!safe(&trust).maths_links());
}

#[test]
fn a_copy_merges_without_asking_only_from_a_trusted_folder_into_a_trusted_document() {
    let trust = trust(SETTINGS);
    let home = |rest: &str| format!("{HOME}/{rest}");
    let cases = [
        (trusted(&trust), home("Documents/report (conflicted copy).let"), MergeDecision::Merge),
        // The copy carries the document's id, so the id cannot vouch for it.
        (trusted(&trust), home("Downloads/report.let"), MergeDecision::Ask),
        (safe(&trust), home("Documents/report (conflicted copy).let"), MergeDecision::Ask),
        (safe(&trust), home("Downloads/report.let"), MergeDecision::Ask),
    ];
    for (gate, copy, decision) in cases {
        assert_eq!(gate.merge(&copy), decision, "{copy} into a {:?} document", gate.mode());
    }
}
